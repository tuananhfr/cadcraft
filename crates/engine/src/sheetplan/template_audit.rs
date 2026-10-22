use std::collections::{BTreeMap, BTreeSet};

use cadcraft_doc::{EntityKind, dynblock};
use cadcraft_fonts::{FontState, font_info};
use cadcraft_render::paper::PAPER_SIZES;
use serde::{Deserialize, Serialize};

use super::codes::{self, issue};
use super::registry_store::save_registry;
use super::{AttributeTags, FrameRegistry, FrameTemplate, GeometryRule, MAX_TEMPLATES, PreflightIssue, Severity, TemplateSource, TemplateStatus};
use crate::hash::sha256_hex;

pub const MAX_AUDIT_BLOCKS: usize = 10_000;
pub const MAX_TEMPLATE_BYTES: usize = 64 * 1024 * 1024;
const MAX_AUDIT_ENTITIES: usize = 200_000;
const MAX_AUDIT_TEXT: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AuditReport {
    pub file_name: String,
    pub sha256: String,
    pub dwg_version: Option<String>,
    pub insunits: i64,
    pub blocks: Vec<BlockAudit>,
    pub layouts: Vec<LayoutAudit>,
    pub fonts_missing: Vec<String>,
    pub issues: Vec<PreflightIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BlockAudit {
    pub name: String,
    pub dynamic: bool,
    pub anonymous_refs: u32,
    pub attdefs: Vec<(String, String)>,
    pub closed_outlines: Vec<[f64; 2]>,
    pub suggested_rule: Option<GeometryRule>,
    pub paper_match: Option<(String, bool)>,
    pub inserts_in_model: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LayoutAudit {
    pub name: String,
    pub paper_mm: [f64; 2],
    pub landscape: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImportPick {
    pub block: String,
    pub template_id: String,
    pub sheet_id_tags: Vec<String>,
    pub revision_tags: Vec<String>,
    pub revision: String,
}

fn limit() -> String {
    format!("{}: template exceeds audit limits", codes::AUDIT_LIMIT)
}

pub fn audit_template(bytes: &[u8], name: &str) -> Result<AuditReport, String> {
    if bytes.len() > MAX_TEMPLATE_BYTES || name.len() > MAX_AUDIT_TEXT {
        return Err(limit());
    }
    let hooks = crate::cmd::file::io().ok_or_else(|| "file formats are not available in this build".to_string())?;
    let drawing = std::panic::catch_unwind(|| (hooks.read)(bytes, name)).map_err(|_| "template file could not be read".to_string())??;
    let d = &drawing;
    if d.blocks.len() > MAX_AUDIT_BLOCKS
        || d.layouts.len() > MAX_AUDIT_BLOCKS
        || d.text_styles.len() > MAX_AUDIT_BLOCKS
        || d.foreign.objects.len() > MAX_AUDIT_ENTITIES
        || d.foreign.records.len() > MAX_AUDIT_ENTITIES
    {
        return Err(limit());
    }
    let mut work = d.model.len();
    for count in d.blocks.values().map(|block| block.entities.len()).chain(d.layouts.iter().map(|layout| layout.entities.len())) {
        work = work.checked_add(count).ok_or_else(limit)?;
    }
    if work > MAX_AUDIT_ENTITIES {
        return Err(limit());
    }
    // Dynamic-name lookups build a foreign-object index; cap their combined work as well as entity counts.
    let lookups = d.model.len().saturating_add(d.blocks.len());
    if lookups.saturating_mul(d.foreign.objects.len()) > MAX_AUDIT_ENTITIES {
        return Err(limit());
    }
    let mut counts: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    for entity in d.model.iter() {
        if let EntityKind::Insert(insert) = &entity.kind {
            let name = dynblock::effective_name(d, entity.handle).unwrap_or_else(|| insert.block.clone());
            let count = counts.entry(name.to_ascii_lowercase()).or_default();
            let instances = insert.cols.max(1).checked_mul(insert.rows.max(1)).ok_or_else(limit)?;
            count.0 = count.0.checked_add(instances).ok_or_else(limit)?;
            if insert.block.starts_with('*') {
                count.1 = count.1.checked_add(instances).ok_or_else(limit)?;
            }
        }
    }
    let (unit_mm, _) = super::plan::drawing_unit_mm(d);
    let mut blocks = Vec::new();
    for block in d.blocks.values() {
        let mut attdefs = Vec::new();
        let mut outlines = Vec::new();
        for entity in block.entities.iter() {
            if let EntityKind::AttDef(attribute) = &entity.kind {
                attdefs.push((attribute.tag.clone(), attribute.prompt.clone()));
            }
            if entity.common.visible
                && d.layer(&entity.common.layer).is_none_or(cadcraft_doc::Layer::visible)
                && let Some(corners) = super::detect::rectangle(entity)
            {
                let [a, b, c, _] = corners;
                outlines.push(([(b - a).len(), (c - b).len()], corners));
            }
        }
        outlines.sort_by(|(a, _), (b, _)| {
            let [aw, ah] = *a;
            let [bw, bh] = *b;
            (bw * bh).total_cmp(&(aw * ah))
        });
        let suggested_rule = outlines.first().map(|(_, corners)| {
            let [a, b, c, e] = *corners;
            let axis_aligned = [b - a, c - b, e - c, a - e].into_iter().all(|v| v.x.abs().min(v.y.abs()) < 1e-6);
            if axis_aligned {
                let min = corners.iter().fold(a, |p, q| cadcraft_geom::Vec2::new(p.x.min(q.x), p.y.min(q.y)));
                let max = corners.iter().fold(a, |p, q| cadcraft_geom::Vec2::new(p.x.max(q.x), p.y.max(q.y)));
                GeometryRule::LocalRect { min: [min.x, min.y], max: [max.x, max.y] }
            } else {
                GeometryRule::LargestClosedRect
            }
        });
        let paper_match = outlines.first().and_then(|([w, h], _)| {
            PAPER_SIZES
                .iter()
                .find(|paper| super::paper::matches_size([paper.width_mm, paper.height_mm], [w.min(*h) * unit_mm, w.max(*h) * unit_mm]))
                .map(|paper| (paper.aliases.first().copied().unwrap_or(paper.name).to_string(), w > h))
        });
        let (inserts_in_model, anonymous_refs) = counts.get(&block.name.to_ascii_lowercase()).copied().unwrap_or_default();
        blocks.push(BlockAudit {
            name: block.name.clone(),
            dynamic: anonymous_refs > 0 || dynblock::dynamic_block(d, &block.name).is_some(),
            anonymous_refs,
            attdefs,
            closed_outlines: outlines.into_iter().map(|(size, _)| size).collect(),
            suggested_rule,
            paper_match,
            inserts_in_model,
        });
    }
    blocks.sort_by(|a, b| a.name.cmp(&b.name));
    let layouts = d
        .layouts
        .iter()
        .map(|layout| LayoutAudit {
            name: layout.name.clone(),
            paper_mm: [layout.page.width_mm, layout.page.height_mm],
            landscape: layout.page.landscape,
        })
        .collect();
    let fonts_missing: Vec<_> = d
        .text_styles
        .iter()
        .filter(|style| font_info(&style.font, &style.family).state == FontState::Missing)
        .map(|style| style.font.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let issues = fonts_missing.iter().map(|font| issue(Severity::Warn, codes::FONT_MISSING, format!("Font {font} is missing."), None)).collect();
    Ok(AuditReport {
        file_name: name.into(),
        sha256: sha256_hex(bytes),
        dwg_version: d.header.get("ACADVER").and_then(cadcraft_doc::HVal::as_str).map(str::to_string),
        insunits: d.header.i64("INSUNITS", 0),
        blocks,
        layouts,
        fonts_missing,
        issues,
    })
}

pub fn import_baseline(reg: &mut FrameRegistry, report: &AuditReport, picks: &[ImportPick], now_ms: u64) -> Result<Vec<String>, String> {
    if picks.is_empty() || picks.len() > MAX_TEMPLATES || report.blocks.len() > MAX_AUDIT_BLOCKS || reg.templates.len() > MAX_TEMPLATES {
        return Err("baseline import requires 1..=1000 picks and a complete audit".into());
    }
    if report.sha256.len() != 64
        || !report.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        || report.file_name.len() > MAX_AUDIT_TEXT
        || report.issues.iter().any(|entry| entry.severity == Severity::Block || entry.code == codes::AUDIT_LIMIT)
    {
        return Err("baseline import requires a valid, complete audit".into());
    }
    let mut next = reg.clone();
    let mut changes = Vec::new();
    let mut seen = BTreeSet::new();
    for pick in picks {
        if !seen.insert(&pick.template_id) {
            return Err(format!("duplicate template id {}", pick.template_id));
        }
        if pick.block.len() > MAX_AUDIT_TEXT
            || pick.template_id.len() > MAX_AUDIT_TEXT
            || pick.revision.len() > MAX_AUDIT_TEXT
            || pick.sheet_id_tags.len() > 64
            || pick.revision_tags.len() > 64
            || pick.sheet_id_tags.iter().chain(&pick.revision_tags).any(|tag| tag.trim().is_empty() || tag.len() > MAX_AUDIT_TEXT)
        {
            return Err(limit());
        }
        let block = report.blocks.iter().find(|block| block.name == pick.block).ok_or_else(|| format!("unknown audited block {}", pick.block))?;
        let rule = block.suggested_rule.clone().ok_or_else(|| format!("no frame rule for {}", pick.block))?;
        if next
            .templates
            .iter()
            .any(|template| template.id == pick.template_id && template.source.as_ref().is_some_and(|source| source.sha256 == report.sha256))
        {
            changes.push(format!("{} unchanged", pick.template_id));
            continue;
        }
        let version = next
            .templates
            .iter()
            .filter(|template| template.id == pick.template_id)
            .map(|template| template.version)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| "template version overflow".to_string())?;
        let template = FrameTemplate {
            id: pick.template_id.clone(),
            revision: pick.revision.clone(),
            block_names: vec![pick.block.clone()],
            geometry: vec![rule],
            attributes: AttributeTags { sheet_id: pick.sheet_id_tags.clone(), revision: pick.revision_tags.clone(), title: Vec::new() },
            media_mm: Vec::new(),
            superseded_by: None,
            version,
            status: TemplateStatus::Approved,
            source: Some(TemplateSource { file_name: report.file_name.clone(), sha256: report.sha256.clone(), imported_ms: now_ms }),
        };
        template.validate()?;
        if next.templates.len() >= MAX_TEMPLATES {
            return Err("baseline import exceeds 1000 templates".into());
        }
        let versioned_id = template.versioned_id();
        for previous in next.templates.iter_mut().filter(|previous| previous.id == pick.template_id && previous.status != TemplateStatus::Superseded)
        {
            previous.status = TemplateStatus::Superseded;
            previous.superseded_by = Some(versioned_id.clone());
        }
        next = next.merged_with(FrameRegistry { schema_version: next.schema_version.clone(), templates: vec![template] });
        changes.push(versioned_id);
    }
    FrameRegistry::from_json(&save_registry(&next))?;
    *reg = next;
    Ok(changes)
}

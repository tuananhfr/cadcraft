use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Value, json};

use super::{CLOCK, bad, registry_notice};
use crate::cmd::file::{base64_decode, store_file};
use crate::hash::sha256_hex;
use crate::sheetplan::codes::{self, issue};
use crate::sheetplan::plan::validate_request_value;
use crate::sheetplan::registry_store::{parse_registry, save_registry};
use crate::sheetplan::template_audit::{ImportPick, MAX_TEMPLATE_BYTES, audit_template, import_baseline};
use crate::sheetplan::{FrameRegistry, GeometryRule, MAX_TEMPLATES, Severity, TemplateStatus};
use crate::{Result, Session};

const COMMAND: &str = "ubplot.templates";

pub(super) fn run_templates(s: &mut Session, params: &Value) -> Result<Value> {
    validate_request_value(params).map_err(|error| bad(COMMAND, error))?;
    let action = match params.get("action") {
        None => "list",
        Some(Value::String(action)) => action.as_str(),
        Some(_) => return Err(bad(COMMAND, "action must be a string")),
    };
    registry_notice(s);
    match action {
        "audit" => {
            let (name, bytes) = input_bytes(params)?;
            let report = audit_template(&bytes, &name).map_err(|error| bad(COMMAND, error))?;
            return serde_json::to_value(report).map_err(|error| bad(COMMAND, error.to_string()));
        }
        "verify" => return Ok(verify(&s.ubplot.registry)),
        "exportLisp" => {
            let text = export_lisp(&s.ubplot.registry);
            if let Some(value) = params.get("path") {
                let path = value.as_str().filter(|path| !path.is_empty()).ok_or_else(|| bad(COMMAND, "path must be a nonempty string"))?;
                protect_sources(s, path)?;
                store_file(COMMAND, path, text.as_bytes())?;
            }
            return Ok(json!({"text": text}));
        }
        _ => {}
    }
    let mut notices = Vec::new();
    let mut changes = Vec::new();
    let next = match action {
        "list" => None,
        "reset" => Some(FrameRegistry::builtin()),
        "import" if params.get("picks").is_some() => {
            if params.get("approve").and_then(Value::as_bool) != Some(true) {
                return Err(bad(COMMAND, "baseline import requires approve: true"));
            }
            let input = params.get("picks").and_then(Value::as_array).ok_or_else(|| bad(COMMAND, "picks must be an array"))?;
            if input.is_empty() || input.len() > MAX_TEMPLATES {
                return Err(bad(COMMAND, "baseline import requires 1..=1000 picks"));
            }
            let picks: Vec<_> =
                input.iter().map(ImportPick::deserialize).collect::<std::result::Result<_, _>>().map_err(|error| bad(COMMAND, error.to_string()))?;
            let expected = params.get("sha256").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "sha256 is required for baseline import"))?;
            let (name, bytes) = input_bytes(params)?;
            if sha256_hex(&bytes) != expected {
                return Err(bad(COMMAND, "sha256 does not match the audited source"));
            }
            let report = audit_template(&bytes, &name).map_err(|error| bad(COMMAND, error))?;
            let mut next = s.ubplot.registry.clone();
            changes = import_baseline(&mut next, &report, &picks, CLOCK.get().map_or(0, |clock| clock())).map_err(|error| bad(COMMAND, error))?;
            (next != s.ubplot.registry).then_some(next)
        }
        "import" => {
            let text = params.get("text").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "text is required for import"))?;
            let (imported, notice) = parse_registry(text, true).map_err(|error| bad(COMMAND, error))?;
            notices.extend(notice);
            let merged = s.ubplot.registry.merged_with(imported);
            // Repeated small imports must obey the same registry limits as one import.
            let (merged, notice) = parse_registry(&save_registry(&merged), true).map_err(|error| bad(COMMAND, error))?;
            notices.extend(notice);
            Some(merged)
        }
        _ => return Err(bad(COMMAND, "action must be list, import, reset, audit, verify or exportLisp")),
    };
    if let Some(next) = next {
        if let Some(store) = s.ubplot.registry_store.as_mut() {
            store.write(&save_registry(&next)).map_err(|error| bad(COMMAND, format!("could not save frame template registry: {error}")))?;
        }
        s.ubplot.registry = next;
    }
    for notice in notices {
        s.echo(notice);
    }
    let templates: Vec<_> = s
        .ubplot
        .registry
        .templates
        .iter()
        .take(MAX_TEMPLATES)
        .map(|template| {
            json!({"id": template.id, "version": template.version, "versionedId": template.versioned_id(), "status": template.status,
                "revision": template.revision, "blockNames": template.block_names, "source": template.source, "supersededBy": template.superseded_by})
        })
        .collect();
    let mut result = json!({"templates": templates});
    if !changes.is_empty()
        && let Some(object) = result.as_object_mut()
    {
        object.insert("changes".into(), json!(changes));
    }
    Ok(result)
}

pub(super) fn run_report(s: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "ubplot.report";
    validate_request_value(params).map_err(|error| bad(COMMAND, error))?;
    let format = match params.get("format") {
        None => "json",
        Some(Value::String(format)) if matches!(format.as_str(), "json" | "csv") => format.as_str(),
        Some(_) => return Err(bad(COMMAND, "format must be json or csv")),
    };
    let value = params.get("plan").ok_or_else(|| bad(COMMAND, "plan is required"))?;
    let plan = crate::sheetplan::parse_plan(value).map_err(|error| bad(COMMAND, error))?;
    let mut rows = Vec::with_capacity(plan.sheets.len());
    let mut csv = String::from("\u{feff}order,sheet_id,revision,frame_template_id,status,issue_codes\r\n");
    for sheet in &plan.sheets {
        let revision = crate::sheetplan::preflight::sheet_revision(sheet, &s.ubplot.registry);
        let codes: Vec<_> = sheet.issues.iter().map(|issue| issue.code.as_str()).collect();
        if format == "csv" {
            let status = serde_json::to_value(sheet.status).map_err(|error| bad(COMMAND, error.to_string()))?;
            let fields = [
                sheet.order.to_string(),
                sheet.sheet_id.clone(),
                revision.unwrap_or("").to_string(),
                sheet.frame_template_id.as_deref().unwrap_or("").to_string(),
                status.as_str().unwrap_or("").to_string(),
                codes.join(";"),
            ];
            csv.push_str(&fields.iter().map(|field| csv_field(field)).collect::<Vec<_>>().join(","));
            csv.push_str("\r\n");
        } else {
            rows.push(json!({"order": sheet.order, "sheet_id": sheet.sheet_id, "revision": revision,
                "frame_template_id": sheet.frame_template_id, "status": sheet.status, "issue_codes": codes}));
        }
    }
    Ok(if format == "csv" { json!({"text": csv}) } else { json!({"rows": rows}) })
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.to_string() }
}

fn input_bytes(params: &Value) -> Result<(String, Vec<u8>)> {
    if let Some(value) = params.get("path") {
        let path = value.as_str().filter(|path| !path.is_empty()).ok_or_else(|| bad(COMMAND, "path must be a nonempty string"))?;
        return Ok((path.into(), read_path(path).map_err(|error| bad(COMMAND, error))?));
    }
    let name = params.get("name").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "name is required with data"))?;
    let data = params.get("data").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "path or name+data (base64) is required"))?;
    if data.len() > MAX_TEMPLATE_BYTES.saturating_add(2) / 3 * 4 {
        return Err(bad(COMMAND, "template exceeds 64 MiB"));
    }
    let bytes = base64_decode(data).ok_or_else(|| bad(COMMAND, "invalid base64"))?;
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err(bad(COMMAND, "template exceeds 64 MiB"));
    }
    Ok((name.into(), bytes))
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path(path: &str) -> std::result::Result<Vec<u8>, String> {
    use std::io::Read;

    let file = std::fs::File::open(path).map_err(|error| format!("{path}: {error}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_TEMPLATE_BYTES as u64 + 1).read_to_end(&mut bytes).map_err(|error| format!("{path}: {error}"))?;
    if bytes.len() > MAX_TEMPLATE_BYTES {
        return Err("template exceeds 64 MiB".into());
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn read_path(_path: &str) -> std::result::Result<Vec<u8>, String> {
    Err("paths are not available on the web; pass name+data".into())
}

fn verify(registry: &FrameRegistry) -> Value {
    let mut hashes = BTreeMap::new();
    let mut issues = Vec::new();
    for template in registry.templates.iter().take(MAX_TEMPLATES) {
        let Some(source) = &template.source else { continue };
        let path = &source.file_name;
        #[cfg(not(target_arch = "wasm32"))]
        if !std::path::Path::new(path).exists() && !path.contains(['/', '\\']) {
            continue;
        }
        #[cfg(target_arch = "wasm32")]
        if !path.contains(['/', '\\']) {
            continue;
        }
        let hash = hashes.entry(path).or_insert_with(|| read_path(path).map(|bytes| sha256_hex(&bytes)));
        let id = template.versioned_id();
        let message = match hash {
            Ok(hash) if *hash == source.sha256 => continue,
            Ok(_) => format!("Baseline source {path} for {id} has changed since import."),
            Err(error) => format!("Baseline source {path} for {id} could not be verified: {error}"),
        };
        issues.push(issue(Severity::Warn, codes::BASELINE_CHANGED, message, None));
    }
    json!({"issues": issues})
}

fn lisp_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t"))
}

fn lisp_strings(values: &[String]) -> String {
    format!("({})", values.iter().map(|value| lisp_string(value)).collect::<Vec<_>>().join(" "))
}

fn export_lisp(registry: &FrameRegistry) -> String {
    let mut entries = Vec::new();
    for template in registry.templates.iter().take(MAX_TEMPLATES) {
        if template.status != TemplateStatus::Approved
            || registry
                .templates
                .iter()
                .take(MAX_TEMPLATES)
                .any(|other| other.id == template.id && other.status == TemplateStatus::Approved && other.version > template.version)
        {
            continue;
        }
        let geometry: Vec<_> = template
            .geometry
            .iter()
            .map(|rule| match rule {
                GeometryRule::LocalRect { min, max } => {
                    let [x0, y0] = *min;
                    let [x1, y1] = *max;
                    format!("(local_rect (min {x0} {y0}) (max {x1} {y1}))")
                }
                GeometryRule::LayerRect { layer } => format!("(layer_rect {})", lisp_string(layer)),
                GeometryRule::LargestClosedRect => "(largest_closed_rect)".into(),
                GeometryRule::EnclosingRect => "(enclosing_rect)".into(),
            })
            .collect();
        for block in &template.block_names {
            entries.push(format!(
                "  ({} (id . {}) (geometry {}) (sheet_id_tags {}) (revision_tags {}) (revision . {}))",
                lisp_string(block),
                lisp_string(&template.versioned_id()),
                geometry.join(" "),
                lisp_strings(&template.attributes.sheet_id),
                lisp_strings(&template.attributes.revision),
                lisp_string(&template.revision)
            ));
        }
    }
    format!("(setq *ubb-templates* '(\n{}\n))\n", entries.join("\n"))
}

fn protect_sources(s: &Session, path: &str) -> Result<()> {
    let temporary = format!("{path}.cadcraft-tmp");
    let sources = s
        .ubplot
        .registry
        .templates
        .iter()
        .filter_map(|template| template.source.as_ref().map(|source| source.file_name.as_str()))
        .chain(s.docs.iter().filter_map(|state| state.path.as_deref()));
    for source in sources {
        for output in [path, temporary.as_str()] {
            if same_path(source, output) {
                return Err(bad(COMMAND, format!("{}: export path is a source drawing or template", codes::OUTPUT_IS_SOURCE)));
            }
        }
    }
    Ok(())
}

fn same_path(a: &str, b: &str) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return a == b;
    }
    a == b
}

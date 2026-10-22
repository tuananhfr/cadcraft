use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use cadcraft_doc::{Handle, Space};
use serde_json::{Value, json};

use super::{LastPlan, bad, io, source_hash};
use crate::cmd::file::base64_encode;
use crate::sheetplan::codes::{self, issue};
use crate::sheetplan::ids::{sha256_hex, sheet_key};
use crate::sheetplan::paper::sheet_margins;
use crate::sheetplan::{PlotScale, ScanMode, ScanRequest, Severity, Sheet, SheetPlan, SheetSource, SheetStatus, parse_plan};
use crate::{Result, Session};

const COMMAND: &str = "ubplot.publish";
const MAX_PAGES: usize = 256;

pub(super) fn run_publish(s: &mut Session, params: &Value) -> Result<Value> {
    crate::sheetplan::plan::validate_request_value(params).map_err(|error| bad(COMMAND, error))?;
    let output = text(params, "output")?.unwrap_or("merged");
    if !matches!(output, "merged" | "split") {
        return Err(bad(COMMAND, "output must be merged or split"));
    }
    let path = text(params, "path")?;
    if path.is_some_and(|path| path.is_empty() || path.len() > 4096) {
        return Err(bad(COMMAND, "path must contain 1 to 4096 bytes"));
    }
    let title = text(params, "title")?.unwrap_or("");
    let supplied = params.get("plan");
    let mut plan = match supplied {
        Some(value) => parse_plan(value).map_err(|error| bad(COMMAND, error))?,
        None => {
            let uid = s.state()?.uid;
            s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid).ok_or_else(|| bad(COMMAND, "no previous UBPLOT plan"))?.plan.clone()
        }
    };
    if source_hash(s)? != plan.source_hash {
        return Err(bad(COMMAND, "PLAN_STALE: drawing changed since scan; scan again"));
    }
    if plan.sheets.is_empty() {
        return Err(bad(COMMAND, "PLAN_INVALID: there are no sheets to publish"));
    }
    if plan.job_id.is_empty() || plan.job_id.len() > 128 || !plan.job_id.bytes().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'-' | b'_')) {
        return Err(bad(COMMAND, "PLAN_INVALID: job_id is not safe for a temporary file name"));
    }
    plan.sheets.sort_by_key(|sheet| sheet.order);
    if output == "merged" && plan.sheets.len() > MAX_PAGES {
        return Err(bad(COMMAND, format!("at most {MAX_PAGES} pages fit in merged output")));
    }
    #[cfg(target_arch = "wasm32")]
    if output == "split" && plan.sheets.len() > 20 {
        return Err(bad(COMMAND, "web split output is limited to 20 files; use merged output"));
    }
    for sheet in &mut plan.sheets {
        if sheet.issues.iter().chain(&plan.issues).any(|entry| entry.severity == Severity::Block) {
            sheet.status = SheetStatus::Blocked;
        }
    }
    let names = split_names(&plan.sheets);
    let base = Path::new(path.unwrap_or("Drawing.pdf"));
    let destinations: Vec<PathBuf> = if output == "merged" {
        vec![base.to_path_buf()]
    } else {
        let stem = base.file_stem().and_then(|name| name.to_str()).unwrap_or("Drawing");
        names.iter().map(|name| base.with_file_name(format!("{stem}-{name}.pdf"))).collect()
    };
    #[cfg(not(target_arch = "wasm32"))]
    if path.is_some()
        && let Some(source) = s.state()?.path.as_deref()
    {
        check_source(base, source)?;
        for destination in &destinations {
            check_source(destination, source)?;
            check_source(&temporary_path(destination, &plan.job_id), source)?;
        }
    }
    let blocked: Vec<_> = plan.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Blocked).map(sheet_key).collect();
    if output == "merged" && !blocked.is_empty() {
        remember(s, &plan, supplied)?;
        return Err(bad(COMMAND, format!("BLOCKED: {}; plan={}", blocked.join(", "), serialize(&plan)?)));
    }
    let plot = io().and_then(|hooks| hooks.plot).ok_or_else(|| bad(COMMAND, "PDF plotting is not available in this build"))?;
    let mut files = Vec::new();
    if output == "merged" {
        let rendered = plan
            .sheets
            .iter()
            .map(|sheet| page(s, sheet))
            .collect::<std::result::Result<Vec<_>, _>>()
            .and_then(|pages| plot(s.doc().map_err(|error| error.to_string())?, &Space::Model, &options(pages, title)));
        let result = rendered.and_then(|bytes| finish(&destinations, 0, path.is_some(), &plan.job_id, &plan.sheets, &bytes));
        match result {
            Ok(file) => {
                files.push(file);
                for sheet in &mut plan.sheets {
                    sheet.status = SheetStatus::Done;
                }
            }
            Err(error) => {
                for sheet in &mut plan.sheets {
                    failed(sheet, &error);
                }
                remember(s, &plan, supplied)?;
                return Err(bad(COMMAND, format!("{error}; plan={}", serialize(&plan)?)));
            }
        }
    } else {
        for (index, sheet) in plan.sheets.iter_mut().enumerate() {
            if sheet.status == SheetStatus::Blocked {
                continue;
            }
            let result = page(s, sheet)
                .and_then(|page| plot(s.doc().map_err(|error| error.to_string())?, &Space::Model, &options(vec![page], title)))
                .and_then(|bytes| finish(&destinations, index, path.is_some(), &plan.job_id, std::slice::from_ref(sheet), &bytes));
            match result {
                Ok(file) => {
                    sheet.status = SheetStatus::Done;
                    files.push(file);
                }
                Err(error) => failed(sheet, &error),
            }
        }
    }
    let failed: Vec<_> = plan
        .sheets
        .iter()
        .filter(|sheet| matches!(sheet.status, SheetStatus::Failed | SheetStatus::Blocked))
        .map(|sheet| {
            let issues: Vec<_> = sheet.issues.iter().chain(plan.issues.iter().filter(|entry| entry.severity == Severity::Block)).collect();
            json!({"sheetKey": sheet_key(sheet), "issues": issues})
        })
        .collect();
    remember(s, &plan, supplied)?;
    Ok(json!({"plan": plan, "files": files, "failed": failed}))
}

fn text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    params.get(key).map(|value| value.as_str().ok_or_else(|| bad(COMMAND, format!("{key} must be a string")))).transpose()
}

fn serialize(plan: &SheetPlan) -> Result<String> {
    serde_json::to_string(plan).map_err(|error| bad(COMMAND, error.to_string()))
}

fn remember(s: &mut Session, plan: &SheetPlan, supplied: Option<&Value>) -> Result<()> {
    let state = s.state()?;
    let (uid, revision) = (state.uid, state.revision);
    if let Some(last) = &mut s.ubplot.last
        && last.doc_uid == uid
    {
        last.plan = plan.clone();
    } else {
        let request = ScanRequest { mode: ScanMode::Plan, plan: supplied.cloned(), ..Default::default() };
        s.ubplot.last = Some(LastPlan { doc_uid: uid, revision, request, plan: plan.clone() });
    }
    Ok(())
}

fn page(s: &Session, sheet: &Sheet) -> std::result::Result<Value, String> {
    let d = s.doc().map_err(|error| error.to_string())?;
    let [a, b] = sheet.paper_mm;
    let (width, height) = if sheet.landscape { (a.max(b), a.min(b)) } else { (a.min(b), a.max(b)) };
    let mut page = json!({"width": width, "height": height, "landscape": sheet.landscape, "label": sheet.sheet_id});
    let object = page.as_object_mut().ok_or_else(|| "page must be an object".to_string())?;
    match &sheet.source {
        SheetSource::Block { block_handle, outline, .. } => {
            let handle = Handle::parse_hex(block_handle).ok_or_else(|| "invalid block handle".to_string())?;
            let space = s.doc().map_err(|error| error.to_string())?.space_of(handle).ok_or_else(|| "frame block no longer exists".to_string())?;
            object.insert("space".into(), json!(space.tab_name()));
            object.insert("outline".into(), json!(outline));
        }
        SheetSource::Window { outline, .. } => {
            object.insert("space".into(), json!("Model"));
            object.insert("outline".into(), json!(outline));
        }
        SheetSource::Layout { name } => {
            object.insert("layout".into(), json!(name));
        }
    }
    match sheet.plot_scale {
        PlotScale::Fit => {
            object.insert("fit".into(), json!(true));
        }
        PlotScale::Ratio { paper_mm, drawing_units } => {
            let scale = paper_mm / drawing_units / cadcraft_render::paper::paper_unit_mm(d);
            if !scale.is_finite() || scale <= 0.0 {
                return Err("plot scale must be finite and positive after unit conversion".into());
            }
            object.insert("scale".into(), json!(scale));
            object.insert("fit".into(), json!(false));
        }
    }
    let margins = sheet_margins(d, sheet);
    object.insert("margins".into(), json!(margins));
    Ok(page)
}

fn options(pages: Vec<Value>, title: &str) -> Value {
    json!({"pages": pages, "title": title, "output": "pdf"})
}

fn failed(sheet: &mut Sheet, error: &str) {
    sheet.status = SheetStatus::Failed;
    sheet.issues.push(issue(Severity::Block, codes::RENDER_FAILED, error, Some(&sheet.sheet_id)));
}

fn split_names(sheets: &[Sheet]) -> Vec<String> {
    let parts: Vec<_> = sheets
        .iter()
        .map(|sheet| {
            let part: String = sheet
                .sheet_id
                .chars()
                .take(120)
                .map(|ch| if ch.is_alphanumeric() || matches!(ch, ' ' | '-' | '_' | '.') { ch } else { '_' })
                .collect();
            let part = part.trim_matches([' ', '.']);
            if part.is_empty() { "Sheet".into() } else { part.to_string() }
        })
        .collect();
    let mut counts = BTreeMap::<String, usize>::new();
    for part in &parts {
        let count = counts.entry(part.to_lowercase()).or_default();
        *count = count.saturating_add(1);
    }
    let mut used = BTreeSet::new();
    sheets
        .iter()
        .zip(&parts)
        .map(|(sheet, part)| {
            let upper = part.split('.').next().unwrap_or("").to_ascii_uppercase();
            let device = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ["COM", "LPT"].iter().any(|prefix| {
                    upper.strip_prefix(prefix).is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
                });
            let duplicate = counts.get(&part.to_lowercase()).is_some_and(|count| *count > 1);
            let mut name = if device || duplicate { format!("{part}-{}", sheet.order) } else { part.clone() };
            // A generated suffix can collide with another sheet's literal id.
            for _ in 0..=sheets.len() {
                if used.insert(name.to_lowercase()) {
                    break;
                }
                name = format!("{name}-{}", sheet.order);
            }
            name
        })
        .collect()
}

fn finish(
    destinations: &[PathBuf],
    index: usize,
    has_path: bool,
    job_id: &str,
    sheets: &[Sheet],
    bytes: &[u8],
) -> std::result::Result<Value, String> {
    let destination = destinations.get(index).ok_or_else(|| "missing output destination".to_string())?;
    let name = destination.file_name().and_then(|name| name.to_str()).ok_or_else(|| "output needs a file name".to_string())?;
    let mut result = json!({
        "name": name, "sheets": sheets.iter().map(sheet_key).collect::<Vec<_>>(), "bytes": bytes.len(), "sha256": sha256_hex(bytes)
    });
    let object = result.as_object_mut().ok_or_else(|| "file result must be an object".to_string())?;
    if has_path {
        store(destination, job_id, bytes)?;
        #[cfg(not(target_arch = "wasm32"))]
        object.insert("path".into(), json!(destination.to_string_lossy()));
    } else {
        object.insert("data".into(), json!(base64_encode(bytes)));
    }
    Ok(result)
}

#[cfg(not(target_arch = "wasm32"))]
fn temporary_path(path: &Path, job_id: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{job_id}.part"));
    PathBuf::from(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn store(path: &Path, job_id: &str, bytes: &[u8]) -> std::result::Result<(), String> {
    use std::io::Write;

    let temporary = temporary_path(path, job_id);
    // Never truncate an existing temp file belonging to another publisher.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|error| error.to_string())?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| std::fs::rename(&temporary, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

#[cfg(target_arch = "wasm32")]
fn store(path: &Path, _job_id: &str, bytes: &[u8]) -> std::result::Result<(), String> {
    let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| "output needs a file name".to_string())?;
    crate::cmd::file::deliver().ok_or_else(|| "file delivery is not available in this build".to_string())?(name, "application/pdf", bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn check_source(output: &Path, source: &str) -> Result<()> {
    fn normalize(path: &Path) -> std::io::Result<PathBuf> {
        use std::path::Component;
        let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
        let mut result = PathBuf::new();
        for component in absolute.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    result.pop();
                }
                other => {
                    result.push(other.as_os_str());
                    if let Ok(canonical) = std::fs::canonicalize(&result) {
                        result = canonical;
                    }
                }
            }
        }
        Ok(result)
    }
    let output = normalize(output).map_err(|error| bad(COMMAND, error.to_string()))?;
    let source = normalize(Path::new(source)).map_err(|error| bad(COMMAND, error.to_string()))?;
    #[cfg(target_os = "windows")]
    let equal = output.to_string_lossy().to_lowercase() == source.to_string_lossy().to_lowercase();
    #[cfg(not(target_os = "windows"))]
    let equal = output == source;
    if equal { Err(bad(COMMAND, "OUTPUT_IS_SOURCE: output would overwrite the source drawing")) } else { Ok(()) }
}

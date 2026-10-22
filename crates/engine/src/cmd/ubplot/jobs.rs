use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{CLOCK, bad, publish, source_hash};
use crate::cmd::file::{budget_plot, merge_pdf};
use crate::sheetplan::approval::{carry_over, issues, merge_audit, validate_approvals, validate_issue_approvals};
use crate::sheetplan::codes::issue;
use crate::sheetplan::queue::store::{validate_name, validate_output_file_name};
use crate::sheetplan::queue::*;
use crate::sheetplan::{PreflightIssue, Severity, SheetPlan, SheetStatus, parse_plan};
use crate::{EngineError, Result, Session};

const PUBLISH: &str = "ubplot.publish";
const JOBS: &str = "ubplot.jobs";
const FOLDER: &str = "ubplot.folder";
const FOLDER_DEFINITION: &str = "folder.json";
// A manifest omits render geometry and output settings; retain them in the same store transaction.
const JOB_DEFINITION: &str = "job.json";

fn now() -> u64 {
    CLOCK.get().map_or(0, |clock| clock())
}

fn text<'a>(command: &str, params: &'a Value, key: &str) -> Result<Option<&'a str>> {
    params.get(key).map(|value| value.as_str().ok_or_else(|| bad(command, format!("{key} must be a string")))).transpose()
}

fn boolean(command: &str, params: &Value, key: &str) -> Result<bool> {
    match params.get(key) {
        Some(value) => value.as_bool().ok_or_else(|| bad(command, format!("{key} must be a boolean"))),
        None => Ok(false),
    }
}

fn number(command: &str, params: &Value, key: &str, default: u64, min: u64, max: u64) -> Result<u64> {
    let value = match params.get(key) {
        Some(value) => value.as_u64().ok_or_else(|| bad(command, format!("{key} must be an unsigned integer")))?,
        None => default,
    };
    if !(min..=max).contains(&value) {
        return Err(bad(command, format!("{key} must be within {min}..={max}")));
    }
    Ok(value)
}

pub(super) fn run_publish(s: &mut Session, params: &Value) -> Result<Value> {
    if !boolean(PUBLISH, params, "background")? {
        return publish::run_publish(s, params);
    }
    crate::sheetplan::plan::validate_request_value(params).map_err(|error| bad(PUBLISH, error))?;
    let path = text(PUBLISH, params, "path")?;
    if path.is_some_and(|path| path.is_empty() || path.len() > 4096) {
        return Err(bad(PUBLISH, "path must contain 1 to 4096 bytes"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    if let (Some(path), Some(source)) = (path, s.state()?.path.as_deref()) {
        publish::check_source(Path::new(path), source)?;
    }
    let mode = text(PUBLISH, params, "mode")?.or(text(PUBLISH, params, "output")?).unwrap_or("merged");
    let mode = match mode {
        "merged" => OutputMode::Merged,
        "split" => OutputMode::PerSheet,
        _ => return Err(bad(PUBLISH, "mode must be merged or split")),
    };
    let mut output = OutputSpec { mode, ..Default::default() };
    let max_pages = u64::try_from(cadcraft_doc::MAX_PLOT_PAGES).map_err(|error| bad(PUBLISH, error.to_string()))?;
    let pages = number(PUBLISH, params, "pagesPerFile", u64::from(output.pages_per_file), 1, max_pages)?;
    output.pages_per_file = u32::try_from(pages).map_err(|error| bad(PUBLISH, error.to_string()))?;
    let retry_limit = u8::try_from(number(PUBLISH, params, "retryLimit", u64::from(DEFAULT_RETRY_LIMIT), 0, u64::from(MAX_RETRY_LIMIT))?)
        .map_err(|error| bad(PUBLISH, error.to_string()))?;
    if let Some(path) = path {
        let path = Path::new(path);
        let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| bad(PUBLISH, "path needs a file name"))?;
        validate_output_file_name(name).map_err(|error| bad(PUBLISH, error))?;
        output.stem = path.file_stem().and_then(|stem| stem.to_str()).ok_or_else(|| bad(PUBLISH, "path needs a file stem"))?.into();
        output.dir = Some(path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_string_lossy().into());
    }
    let plan = queue_plan(s, params)?;
    let source = JobSource { path: s.state()?.path.clone(), source_hash: plan.source_hash.clone(), plan };
    let mut job = PrintJob::new(vec![source], output, retry_limit).map_err(job_error)?;
    let source = job.sources.first_mut().ok_or_else(|| bad(PUBLISH, "job source is missing"))?;
    prepare_plan(&mut source.plan)?;
    let job = PrintJob::new(job.sources, job.output, job.retry_limit).map_err(job_error)?;
    let mut manifest = PublishManifest::new(&job, now());
    if !s.plot_jobs.persisted {
        manifest.issues.push(issue(Severity::Info, "JOBS_NOT_PERSISTED", "Jobs are kept in memory until a host installs persistent storage.", None));
    }
    let definition = serde_json::to_vec(&job).map_err(|error| bad(PUBLISH, error.to_string()))?;
    if definition.len() > MAX_MANIFEST_BYTES {
        return Err(bad(PUBLISH, "job definition exceeds 64 MiB"));
    }
    let store = s.ensure_job_store();
    if let Some(existing) = store.read_manifest(&job.job_id).map_err(|error| bad(PUBLISH, error))? {
        let existing = PublishManifest::from_json(&existing).map_err(|error| bad(PUBLISH, error))?;
        let saved = store.read_artifact(&job.job_id, JOB_DEFINITION).map_err(|error| bad(PUBLISH, error))?;
        if existing.job_id != job.job_id || saved.as_deref() != Some(definition.as_slice()) {
            return Err(bad(PUBLISH, "jobId already exists with a different plan or settings; remove it before publishing again"));
        }
        return Ok(json!({"jobId": job.job_id, "status": existing.status}));
    }
    if store.list_jobs().len() >= MAX_JOB_SHEETS || s.plot_jobs.queued.len() >= MAX_JOB_SHEETS {
        return Err(bad(PUBLISH, "job queue exceeds supported limits"));
    }
    s.ensure_job_store().commit(&job.job_id, &[(JOB_DEFINITION.into(), definition)], &manifest.to_json()).map_err(|error| bad(PUBLISH, error))?;
    let result = json!({"jobId": job.job_id, "status": "QUEUED"});
    s.plot_jobs.queued.push_back(job);
    Ok(result)
}

#[cfg(target_arch = "wasm32")]
pub(super) fn run_folder(_: &mut Session, _: &Value) -> Result<Value> {
    Err(bad(FOLDER, "Folder mode is only in the desktop app"))
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn run_folder(s: &mut Session, params: &Value) -> Result<Value> {
    use crate::sheetplan::queue::folder::{absolute, folder_source};

    crate::sheetplan::plan::validate_request_value(params).map_err(|error| bad(FOLDER, error))?;
    let folder = text(FOLDER, params, "folder")?.ok_or_else(|| bad(FOLDER, "folder is required"))?;
    let out = text(FOLDER, params, "out")?.ok_or_else(|| bad(FOLDER, "out is required"))?;
    if [folder, out].into_iter().any(|path| path.is_empty() || path.len() > 4096) {
        return Err(bad(FOLDER, "folder and out must contain 1 to 4096 bytes"));
    }
    let recursive = if params.get("recursive").is_some() { boolean(FOLDER, params, "recursive")? } else { true };
    let depth = number(FOLDER, params, "maxDepth", 16, 0, 32)?;
    let count = number(FOLDER, params, "maxFiles", 1000, 1, u64::try_from(MAX_JOB_SOURCES).map_err(|error| bad(FOLDER, error.to_string()))?)?;
    let layouts = boolean(FOLDER, params, "layouts")?;
    let background = boolean(FOLDER, params, "background")?;
    let mode = match text(FOLDER, params, "mode")?.unwrap_or("merged") {
        "merged" => OutputMode::Merged,
        "split" | "per_sheet" => OutputMode::PerSheet,
        _ => return Err(bad(FOLDER, "mode must be merged or split")),
    };
    let pages = number(FOLDER, params, "pagesPerFile", 256, 1, 256)?;
    let root = absolute(Path::new(folder)).map_err(|error| bad(FOLDER, error))?;
    if !root.is_dir() {
        return Err(bad(FOLDER, "folder must be an existing directory"));
    }
    let out = absolute(Path::new(out)).map_err(|error| bad(FOLDER, error))?;
    if out.exists() && !out.is_dir() {
        return Err(bad(FOLDER, "out must be a directory"));
    }
    if out == root {
        return Err(bad(FOLDER, "out must differ from folder"));
    }
    if super::io().is_none() || budget_plot().is_none() || merge_pdf().is_none() {
        return Err(bad(FOLDER, "file decoding and PDF queue rendering are not available in this build"));
    }
    let depth = usize::try_from(depth).map_err(|error| bad(FOLDER, error.to_string()))?;
    let count = usize::try_from(count).map_err(|error| bad(FOLDER, error.to_string()))?;
    let mut scan = scan_folder(&root, if recursive { depth } else { 0 }, count, std::slice::from_ref(&out));
    let mut sources = Vec::new();
    let mut sheets = 0_usize;
    for path in &scan.files {
        match folder_source(path, &s.ubplot.registry, layouts) {
            Ok(mut source) => {
                sheets = sheets.checked_add(source.plan.sheets.len()).ok_or_else(|| bad(FOLDER, "job sheet count overflow"))?;
                if sheets > MAX_JOB_SHEETS {
                    return Err(bad(FOLDER, "job exceeds sheet limits"));
                }
                if !source.plan.sheets.is_empty() {
                    prepare_plan(&mut source.plan).map_err(|error| bad(FOLDER, error.to_string()))?;
                }
                sources.push(source);
            }
            Err(error) => {
                if scan.skipped.len() >= MAX_JOB_SOURCES {
                    return Err(bad(FOLDER, "skipped file count exceeds supported limits"));
                }
                scan.skipped.push(SkippedFile { path: path.clone(), reason: format!("read_error: {error}") });
            }
        }
    }
    if sources.is_empty() {
        return Err(bad(FOLDER, format!("no readable drawings found; skipped_files: {}", json!(scan.skipped))));
    }
    let output = OutputSpec {
        mode,
        dir: Some(out.to_string_lossy().into_owned()),
        pages_per_file: u32::try_from(pages).map_err(|error| bad(FOLDER, error.to_string()))?,
        ..Default::default()
    };
    let job = PrintJob::new(sources, output, DEFAULT_RETRY_LIMIT).map_err(|issues| bad(FOLDER, job_error(issues).to_string()))?;
    let definition = serde_json::to_vec(&job).map_err(|error| bad(FOLDER, error.to_string()))?;
    if definition.len() > MAX_MANIFEST_BYTES {
        return Err(bad(FOLDER, "job definition exceeds 64 MiB"));
    }
    let mut manifest = PublishManifest::new(&job, now());
    scan.skipped.sort_by_key(|entry| (entry.path.to_lowercase(), entry.path.clone()));
    manifest.skipped_files = scan.skipped;
    if !s.plot_jobs.persisted {
        manifest.issues.push(issue(Severity::Info, "JOBS_NOT_PERSISTED", "Jobs are kept in memory until a host installs persistent storage.", None));
    }
    let store = s.ensure_job_store();
    if let Some(existing) = store.read_manifest(&job.job_id).map_err(|error| bad(FOLDER, error))? {
        manifest = PublishManifest::from_json(&existing).map_err(|error| bad(FOLDER, error))?;
        let saved = store.read_artifact(&job.job_id, JOB_DEFINITION).map_err(|error| bad(FOLDER, error))?;
        if manifest.job_id != job.job_id
            || saved.as_deref() != Some(definition.as_slice())
            || store.read_artifact(&job.job_id, FOLDER_DEFINITION).map_err(|error| bad(FOLDER, error))?.is_none()
        {
            return Err(bad(FOLDER, "jobId already exists with a different plan or settings; remove it before publishing again"));
        }
    } else {
        if store.list_jobs().len() >= MAX_JOB_SHEETS || s.plot_jobs.queued.len() >= MAX_JOB_SHEETS {
            return Err(bad(FOLDER, "job queue exceeds supported limits"));
        }
        s.ensure_job_store()
            .commit(&job.job_id, &[(JOB_DEFINITION.into(), definition), (FOLDER_DEFINITION.into(), b"true".to_vec())], &manifest.to_json())
            .map_err(|error| bad(FOLDER, error))?;
    }
    if background {
        let result = json!({"jobId": job.job_id, "status": manifest.status, "skipped_files": manifest.skipped_files});
        if !matches!(manifest.status, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled)
            && !s.plot_jobs.queued.iter().any(|queued| queued.job_id == job.job_id)
        {
            s.plot_jobs.queued.push_back(job);
        }
        return Ok(result);
    }
    let id = job.job_id.clone();
    let manifest = run_to_end(job, s.ensure_job_store(), &mut FolderLoader::default(), &PdfSheetRenderer, &SheetBudget::desktop(), &now)
        .map_err(|error| bad(FOLDER, error))?;
    s.plot_jobs.queued.retain(|job| job.job_id != id);
    s.plot_jobs.cancel_requests.retain(|request| request != &id);
    Ok(json!({"skipped_files": manifest.skipped_files, "manifest": manifest}))
}

fn queue_plan(s: &mut Session, params: &Value) -> Result<SheetPlan> {
    let uid = s.state()?.uid;
    let mut plan = match params.get("plan") {
        Some(value) => parse_plan(value).map_err(|error| bad(PUBLISH, error))?,
        None => s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid).ok_or_else(|| bad(PUBLISH, "no previous UBPLOT plan"))?.plan.clone(),
    };
    if let Some(last) = s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid) {
        plan.audit = merge_audit(&last.plan.audit, &plan.audit).map_err(|error| bad(PUBLISH, error))?;
    }
    if source_hash(s)? != plan.source_hash {
        return Err(bad(PUBLISH, "PLAN_STALE: drawing changed since scan; scan again"));
    }
    if plan.sheets.is_empty() {
        return Err(bad(PUBLISH, "PLAN_INVALID: there are no sheets to publish"));
    }
    plan.sheets.sort_by_key(|sheet| sheet.order);
    Ok(plan)
}

fn job_error(issues: Vec<PreflightIssue>) -> EngineError {
    bad(PUBLISH, issues.iter().map(|issue| format!("{}: {}", issue.code, issue.message)).collect::<Vec<_>>().join("; "))
}

fn prepare_plan(plan: &mut SheetPlan) -> Result<()> {
    validate_issue_approvals(plan).map_err(|error| bad(PUBLISH, error))?;
    let previous = plan.clone();
    let device = plan.device.as_deref().and_then(crate::cmd::plotters::find_device);
    let missing = plan.device.is_some() && device.is_none();
    crate::sheetplan::preflight::check_devices(plan, device.as_ref(), missing, cfg!(target_arch = "wasm32"));
    carry_over(&previous, plan);
    validate_approvals(plan).map_err(|error| bad(PUBLISH, error))?;
    let warnings: Vec<_> = issues(plan)
        .filter(|(_, issue)| issue.severity == Severity::Warn && !issue.approved_by_user)
        .map(|(key, issue)| format!("{key}:{}", issue.code))
        .collect();
    if !warnings.is_empty() {
        return Err(bad(PUBLISH, format!("WARN_NOT_APPROVED: {}", warnings.join(", "))));
    }
    Ok(())
}

fn read_manifest(store: &dyn JobStore, id: &str) -> Result<PublishManifest> {
    let text = store.read_manifest(id).map_err(|error| bad(JOBS, error))?.ok_or_else(|| bad(JOBS, "unknown jobId"))?;
    let manifest = PublishManifest::from_json(&text).map_err(|error| bad(JOBS, error))?;
    if manifest.job_id != id {
        return Err(bad(JOBS, "stored manifest does not match jobId"));
    }
    Ok(manifest)
}

fn read_job(store: &dyn JobStore, id: &str) -> Result<PrintJob> {
    let bytes =
        store.read_artifact(id, JOB_DEFINITION).map_err(|error| bad(JOBS, error))?.ok_or_else(|| bad(JOBS, "stored job definition is missing"))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(bad(JOBS, "job definition exceeds 64 MiB"));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| bad(JOBS, error.to_string()))?;
    crate::sheetplan::types::validate_input_size(&value).map_err(|error| bad(JOBS, error))?;
    let sources = value.get("sources").and_then(Value::as_array).ok_or_else(|| bad(JOBS, "job sources must be an array"))?;
    if sources.len() > MAX_JOB_SOURCES {
        return Err(bad(JOBS, "job exceeds source limits"));
    }
    for source in sources {
        parse_plan(source.get("plan").ok_or_else(|| bad(JOBS, "source plan is missing"))?).map_err(|error| bad(JOBS, error))?;
    }
    let job: PrintJob = serde_json::from_value(value).map_err(|error| bad(JOBS, error.to_string()))?;
    if job.job_id != id {
        return Err(bad(JOBS, "stored job definition does not match jobId"));
    }
    Ok(job)
}

fn summary(manifest: &PublishManifest) -> Value {
    let done = manifest.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Done).count();
    let failed = manifest.sheets.iter().filter(|sheet| matches!(sheet.status, SheetStatus::Failed | SheetStatus::Blocked)).count();
    json!({
        "jobId": manifest.job_id, "status": manifest.status, "done": done, "failed": failed,
        "total": manifest.sheets.len(), "updatedMs": manifest.updated_ms,
    })
}

pub(super) fn run_jobs(s: &mut Session, params: &Value) -> Result<Value> {
    crate::sheetplan::plan::validate_request_value(params).map_err(|error| bad(JOBS, error))?;
    if !params.is_object() {
        return Err(bad(JOBS, "parameters must be an object"));
    }
    let action = text(JOBS, params, "action")?.ok_or_else(|| bad(JOBS, "action is required"))?;
    if !matches!(action, "list" | "status" | "manifest" | "resume" | "cancel" | "remove") {
        return Err(bad(JOBS, "unknown job action"));
    }
    let id = text(JOBS, params, "jobId")?;
    if let Some(id) = id {
        validate_name(id).map_err(|error| bad(JOBS, error))?;
    }
    boolean(JOBS, params, "background")?;
    if action == "list" {
        let store = s.ensure_job_store();
        let ids = store.list_jobs();
        if ids.len() > MAX_JOB_SHEETS {
            return Err(bad(JOBS, "job listing exceeds supported limits"));
        }
        let mut jobs = Vec::with_capacity(ids.len());
        for id in ids {
            validate_name(&id).map_err(|error| bad(JOBS, error))?;
            jobs.push(summary(&read_manifest(store, &id)?));
        }
        return Ok(Value::Array(jobs));
    }
    let id = id.ok_or_else(|| bad(JOBS, "jobId is required"))?;
    let mut manifest = read_manifest(s.ensure_job_store(), id)?;
    match action {
        "status" => Ok(summary(&manifest)),
        "manifest" => Ok(json!({"manifest": manifest})),
        "resume" => {
            if budget_plot().is_none() || merge_pdf().is_none() {
                return Err(bad(JOBS, "PDF queue rendering is not available in this build"));
            }
            let job = read_job(s.ensure_job_store(), id)?;
            let folder = s.ensure_job_store().read_artifact(id, FOLDER_DEFINITION).map_err(|error| bad(JOBS, error))?.is_some();
            let mut loader: Box<dyn SourceLoader> = if folder {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    Box::new(FolderLoader::default())
                }
                #[cfg(target_arch = "wasm32")]
                {
                    return Err(bad(JOBS, "Folder mode is only in the desktop app"));
                }
            } else {
                Box::new(SnapshotLoader(Arc::new(s.doc()?.clone()), source_hash(s)?))
            };
            #[cfg(not(target_arch = "wasm32"))]
            let budget = SheetBudget::desktop();
            #[cfg(target_arch = "wasm32")]
            let budget = SheetBudget::web();
            let store = s.ensure_job_store();
            let manifest = run_to_end(job, store, loader.as_mut(), &PdfSheetRenderer, &budget, &now).map_err(|error| bad(JOBS, error))?;
            s.plot_jobs.queued.retain(|job| job.job_id != id);
            s.plot_jobs.cancel_requests.retain(|request| request != id);
            Ok(json!({"manifest": manifest}))
        }
        "cancel" => {
            if !matches!(manifest.status, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled) {
                let requested = s.plot_jobs.cancel_requests.iter().any(|request| request == id);
                if !requested && s.plot_jobs.cancel_requests.len() >= MAX_JOB_SHEETS {
                    return Err(bad(JOBS, "cancel requests exceed supported limits"));
                }
                if manifest.status == JobStatus::Queued {
                    manifest.status = JobStatus::Cancelled;
                    manifest.updated_ms = now();
                    s.ensure_job_store().commit(id, &[], &manifest.to_json()).map_err(|error| bad(JOBS, error))?;
                    s.plot_jobs.queued.retain(|job| job.job_id != id);
                }
                if !requested {
                    s.plot_jobs.cancel_requests.push(id.into());
                }
            }
            Ok(json!({"jobId": id, "status": manifest.status}))
        }
        "remove" => {
            if manifest.status == JobStatus::Running {
                return Err(bad(JOBS, "cannot remove a RUNNING job"));
            }
            s.ensure_job_store().remove_job(id).map_err(|error| bad(JOBS, error))?;
            s.plot_jobs.queued.retain(|job| job.job_id != id);
            s.plot_jobs.cancel_requests.retain(|request| request != id);
            Ok(json!({"jobId": id, "removed": true}))
        }
        _ => Err(bad(JOBS, "unknown job action")),
    }
}

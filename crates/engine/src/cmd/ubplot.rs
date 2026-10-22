mod interactive;
pub(crate) mod publish;

use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{Value, json};

use super::file::io;
use super::{CommandSpec, has_doc};
use crate::sheetplan::approval::{MAX_APPROVALS, approve_with_splits, carry_over, merge_audit};
use crate::sheetplan::codes::{self, issue};
use crate::sheetplan::ids::sha256_hex;
use crate::sheetplan::plan::{build_plan_with_context, validate_request_value};
use crate::sheetplan::preflight::{PreflightContext, preflight, styles_on_sheets, verdict};
use crate::sheetplan::{FrameRegistry, ScanMode, ScanRequest, Severity, SheetPlan};
use crate::{EngineError, Result, Session};

pub type ClockHook = fn() -> u64;

static CLOCK: OnceLock<ClockHook> = OnceLock::new();

pub fn set_clock(h: ClockHook) {
    let _ = CLOCK.set(h);
}

pub struct UbplotState {
    pub registry: FrameRegistry,
    pub last: Option<LastPlan>,
    pub review: bool,
    hash_cache: Option<(u64, u64, String)>,
}

impl Default for UbplotState {
    fn default() -> Self {
        Self { registry: FrameRegistry::builtin(), last: None, review: false, hash_cache: None }
    }
}

pub struct LastPlan {
    pub doc_uid: u64,
    pub revision: u64,
    pub request: ScanRequest,
    pub plan: SheetPlan,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("ubplot", "Quick Plot...", run_ubplot)
            .alias(&["ubp"])
            .menu(&["File", "Quick Plot..."])
            .params("as ubplot.scan (JSON: scans and stores the plan, opens nothing); typed: prompts")
            .noundo()
            .interactive(|_| Ok(Box::new(interactive::UbplotM::default()))),
        CommandSpec::new("ubplot.approve", "Approve Plot Warnings", run_approve)
            .params("{plan?: SheetPlan (default: the last plan), approvals?: [{sheetKey, code}], split?: [{sheetKey, overlapMm?}], by: user|agent:<name>} → SheetPlan; WARN approvals and explicit device splits are atomic")
            .noundo()
            .enabled(has_doc),
        CommandSpec::new("ubplot.publish", "Publish Plot Frames", publish::run_publish)
            .params("{plan?: SheetPlan (default: the last plan), output?: merged|split (merged), path?: file (merged) or file whose stem names split files, title?} → {plan, files: [{path?|name, sheets: [sheet_key], bytes, sha256, data?}], failed: [{sheetKey, issues}]}")
            .noundo()
            .enabled(has_doc),
        CommandSpec::new("ubplot.scan", "Scan Plot Frames", run_scan)
            .params("{mode: block|window|layout|last|plan, handles?, windows?: [[[x1,y1],[x2,y2]] | [[x,y]×4]], layouts?, paper?, scale?: \"1:100\"|\"fit\", device?: name, revisions?: {sheet_id: revision}, overrides?: {sheet_key: {paper?, scale?, window?, sheetId?}}, exclude?: [sheet_key], plan? (mode plan)} → SheetPlan + summary")
            .noundo()
            .enabled(has_doc),
        CommandSpec::new("ubplot.templates", "Plot Frame Templates", run_templates)
            .params("{action: list|import|reset, text?} → {templates:[{id, revision, blockNames}]}")
            .noundo(),
    ]
}

fn bad(command: &str, message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: command.into(), msg: message.into() }
}

fn run_ubplot(s: &mut Session, params: &Value) -> Result<Value> {
    run_scan(s, params)
}

pub(crate) fn run_scan(s: &mut Session, params: &Value) -> Result<Value> {
    validate_request_value(params).map_err(|error| bad("ubplot.scan", error))?;
    let incoming = ScanRequest::deserialize(params).map_err(|error| bad("ubplot.scan", error.to_string()))?;
    let (uid, revision) = {
        let state = s.state()?;
        (state.uid, state.revision)
    };
    let (request, reused, rescanned) = if incoming.mode == ScanMode::Last {
        let last = s
            .ubplot
            .last
            .as_ref()
            .filter(|last| last.doc_uid == uid)
            .ok_or_else(|| bad("ubplot.scan", "no previous UBPLOT plan for this drawing"))?;
        (last.request.clone(), (last.revision == revision).then(|| last.plan.clone()), last.revision != revision)
    } else {
        (incoming, None, false)
    };
    let device_name = request.device.as_deref().or_else(|| reused.as_ref().and_then(|plan| plan.device.as_deref()));
    let device = device_name.and_then(super::plotters::find_device);
    let device_missing = device_name.is_some() && device.is_none();
    let mut plan = if let Some(plan) = reused {
        plan
    } else {
        let hash = source_hash(s)?;
        let ctx = PreflightContext {
            registry: &s.ubplot.registry,
            device: device.as_ref(),
            device_missing,
            revisions: &request.revisions,
            fonts: &[],
            web: cfg!(target_arch = "wasm32"),
        };
        let mut plan = build_plan_with_context(s.doc()?, &request, &ctx, &hash).map_err(|error| bad("ubplot.scan", error))?;
        plan.issues.retain(|entry| entry.code != codes::SOURCE_HASH_WEAK);
        if hash == sha256_hex(format!("{uid}:{revision}").as_bytes()) {
            plan.issues.push(issue(Severity::Info, codes::SOURCE_HASH_WEAK, "Source hash uses only the drawing identity and revision.", None));
        }
        plan
    };
    // A drawing too large to walk still scans; every style then counts as used, so no font warning is lost.
    let used = styles_on_sheets(s.doc()?, &plan).ok();
    let fonts: Vec<_> = super::fonts::style_status(s.doc()?)
        .into_iter()
        .filter(|font| used.as_ref().is_none_or(|used| used.contains(&font.style.to_ascii_lowercase())))
        .collect();
    let ctx = PreflightContext {
        registry: &s.ubplot.registry,
        device: device.as_ref(),
        device_missing,
        revisions: &request.revisions,
        fonts: &fonts,
        web: cfg!(target_arch = "wasm32"),
    };
    let previous = match request.plan.as_ref().filter(|_| request.mode == ScanMode::Plan) {
        Some(value) => {
            let mut previous = crate::sheetplan::parse_plan(value).map_err(|error| bad("ubplot.scan", error))?;
            if let Some(last) = s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid) {
                previous.audit = merge_audit(&last.plan.audit, &previous.audit).map_err(|error| bad("ubplot.scan", error))?;
            }
            previous
        }
        None => s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid).map(|last| &last.plan).unwrap_or(&plan).clone(),
    };
    preflight(s.doc()?, &mut plan, &ctx);
    carry_over(&previous, &mut plan);
    let result = plan_result(&plan, rescanned)?;
    s.ubplot.last = Some(LastPlan { doc_uid: uid, revision, request, plan });
    s.ubplot.review = false;
    Ok(result)
}

fn plan_result(plan: &SheetPlan, rescanned: bool) -> Result<Value> {
    let mut result = serde_json::to_value(plan).map_err(|error| bad("ubplot.scan", error.to_string()))?;
    let (mut pass, mut info, mut warn, mut block) = (0_u64, 0_u64, 0_u64, 0_u64);
    for sheet in &plan.sheets {
        match verdict(sheet) {
            Severity::Pass => pass += 1,
            Severity::Info => info += 1,
            Severity::Warn => warn += 1,
            Severity::Block => block += 1,
        }
    }
    let object = result.as_object_mut().ok_or_else(|| bad("ubplot.scan", "plan must serialize as an object"))?;
    object.insert("summary".into(), json!({"pass": pass, "info": info, "warn": warn, "block": block}));
    object.insert("jobId".into(), json!(plan.job_id));
    if rescanned {
        object.insert("rescanned".into(), Value::Bool(true));
    }
    Ok(result)
}

fn run_approve(s: &mut Session, params: &Value) -> Result<Value> {
    const COMMAND: &str = "ubplot.approve";
    validate_request_value(params).map_err(|error| bad(COMMAND, error))?;
    let by = params.get("by").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "by must be user or agent:<name>"))?;
    let input = match params.get("approvals") {
        Some(value) => value.as_array().ok_or_else(|| bad(COMMAND, "approvals must be an array"))?.as_slice(),
        None => &[],
    };
    let split_input = match params.get("split") {
        Some(value) => value.as_array().ok_or_else(|| bad(COMMAND, "split must be an array"))?.as_slice(),
        None => &[],
    };
    if params.get("approvals").is_none() && params.get("split").is_none() {
        return Err(bad(COMMAND, "approvals or split must be an array"));
    }
    if input.len().saturating_add(split_input.len()) > MAX_APPROVALS {
        return Err(bad(COMMAND, format!("at most {MAX_APPROVALS} approvals are allowed")));
    }
    let mut splits = Vec::with_capacity(split_input.len());
    for item in split_input {
        let key = item.get("sheetKey").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "sheetKey must be a string"))?;
        let overlap = item
            .get("overlapMm")
            .map(|value| {
                value
                    .as_f64()
                    .filter(|value| value.is_finite() && (0.0..=200.0).contains(value))
                    .ok_or_else(|| bad(COMMAND, "overlapMm must be 0..=200 mm"))
            })
            .transpose()?;
        splits.push((key.to_string(), overlap));
    }
    let mut items = Vec::with_capacity(input.len());
    for item in input {
        let key = item.get("sheetKey").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "sheetKey must be a string"))?;
        let code = item.get("code").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "code must be a string"))?;
        items.push((key.to_string(), code.to_string()));
    }
    let supplied = params.get("plan");
    let mut plan = match supplied {
        Some(value) => crate::sheetplan::parse_plan(value).map_err(|error| bad(COMMAND, error))?,
        None => {
            let uid = s.state()?.uid;
            s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid).ok_or_else(|| bad(COMMAND, "no previous UBPLOT plan"))?.plan.clone()
        }
    };
    let uid = s.state()?.uid;
    if let Some(last) = s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid) {
        plan.audit = merge_audit(&last.plan.audit, &plan.audit).map_err(|error| bad(COMMAND, error))?;
    }
    let at_ms = CLOCK.get().map_or(0, |clock| clock());
    approve_with_splits(&mut plan, &items, &splits, by, at_ms).map_err(|error| bad(COMMAND, error))?;
    let result = serde_json::to_value(&plan).map_err(|error| bad(COMMAND, error.to_string()))?;
    publish::remember(s, &plan, supplied)?;
    Ok(result)
}

fn run_templates(s: &mut Session, params: &Value) -> Result<Value> {
    validate_request_value(params).map_err(|error| bad("ubplot.templates", error))?;
    let action = match params.get("action") {
        None => "list",
        Some(Value::String(action)) => action.as_str(),
        Some(_) => return Err(bad("ubplot.templates", "action must be list, import or reset")),
    };
    match action {
        "list" => {}
        "reset" => s.ubplot.registry = FrameRegistry::builtin(),
        "import" => {
            let text = params.get("text").and_then(Value::as_str).ok_or_else(|| bad("ubplot.templates", "text is required for import"))?;
            let imported = FrameRegistry::from_json(text).map_err(|error| bad("ubplot.templates", error))?;
            let merged = s.ubplot.registry.merged_with(imported);
            // Repeated small imports must obey the same registry limits as one import.
            let text = serde_json::to_string(&merged).map_err(|error| bad("ubplot.templates", error.to_string()))?;
            s.ubplot.registry = FrameRegistry::from_json(&text).map_err(|error| bad("ubplot.templates", error))?;
        }
        _ => return Err(bad("ubplot.templates", "action must be list, import or reset")),
    }
    let templates: Vec<_> = s
        .ubplot
        .registry
        .templates
        .iter()
        .take(500)
        .map(|template| json!({"id": template.id, "revision": template.revision, "blockNames": template.block_names}))
        .collect();
    Ok(json!({"templates": templates}))
}

pub(crate) fn source_hash(s: &mut Session) -> Result<String> {
    let state = s.state()?;
    let (uid, revision) = (state.uid, state.revision);
    if let Some((cached_uid, cached_revision, hash)) = &s.ubplot.hash_cache
        && (*cached_uid, *cached_revision) == (uid, revision)
    {
        return Ok(hash.clone());
    }
    #[cfg(not(target_arch = "wasm32"))]
    let saved_hash = if !state.is_dirty() { state.path.as_deref().and_then(hash_file) } else { None };
    #[cfg(target_arch = "wasm32")]
    let saved_hash: Option<String> = None;
    let hash = match saved_hash {
        Some(hash) => hash,
        None => match io() {
            Some(hooks) => sha256_hex(&(hooks.write)(s.doc()?, "plan.dxf").map_err(|error| bad("ubplot.scan", error))?),
            None => sha256_hex(format!("{uid}:{revision}").as_bytes()),
        },
    };
    s.ubplot.hash_cache = Some((uid, revision, hash.clone()));
    Ok(hash)
}

#[cfg(not(target_arch = "wasm32"))]
fn hash_file(path: &str) -> Option<String> {
    use std::io::Read;

    use sha2::{Digest, Sha256};

    let mut file = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    // Stream bounded chunks so hashing a saved drawing does not duplicate it in memory.
    for _ in 0..1_048_576 {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            return Some(format!("{:x}", hash.finalize()));
        }
        hash.update(buffer.get(..count)?);
    }
    None
}

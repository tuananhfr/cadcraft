use serde_json::{Value, json};

use super::{CLOCK, bad};
use crate::sheetplan::profile::{self, EffectiveProfile, ProfileDoc, ProfileLevel};
use crate::{Result, Session};

const COMMAND: &str = "ubplot.profile";

pub fn is_user_only_profile_action(params: &Value) -> bool {
    let action = params.get("action").and_then(Value::as_str).unwrap_or("");
    if !matches!(action, "set" | "unset" | "lock" | "unlock" | "import") {
        return false;
    }
    // Resolve the level with run_profile's parser; anything it cannot read stays with the user.
    let parse = |value: &Value| serde_json::from_value::<ProfileLevel>(value.clone()).ok();
    let level = match params.get("level") {
        Some(value) => parse(value),
        None if action == "import" => params
            .get("text")
            .and_then(Value::as_str)
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|doc| doc.get("level").map_or(Some(ProfileLevel::User), parse)),
        None if matches!(action, "lock" | "unlock") => Some(ProfileLevel::Company),
        None => Some(ProfileLevel::User),
    };
    level.is_none_or(|level| level == ProfileLevel::Company)
}

fn validate_content(value: &Value, depth: usize, remaining: &mut usize) -> std::result::Result<(), String> {
    if depth > profile::MAX_PROFILE_DEPTH {
        return Err("print profile JSON depth exceeds 32".into());
    }
    *remaining = remaining.checked_sub(1).ok_or("print profile exceeds 256 KiB")?;
    match value {
        Value::String(text) => {
            *remaining = remaining.checked_sub(text.len()).ok_or("print profile exceeds 256 KiB")?;
            let lower = text.trim_end().to_ascii_lowercase();
            if [".lsp", ".fas", ".vlx", ".exe", ".dll"].iter().any(|suffix| lower.ends_with(suffix)) {
                return Err("executable paths are forbidden in print profiles".into());
            }
        }
        Value::Object(fields) => {
            for (key, child) in fields {
                *remaining = remaining.checked_sub(key.len()).ok_or("print profile exceeds 256 KiB")?;
                if matches!(key.to_ascii_lowercase().as_str(), "script" | "lisp" | "exec" | "eval" | "code" | "onload" | "startup") {
                    return Err("executable keys are forbidden in print profiles".into());
                }
                validate_content(child, depth + 1, remaining)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_content(child, depth + 1, remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn key(params: &Value) -> Result<String> {
    let key = params.get("key").and_then(Value::as_str).filter(|key| !key.is_empty()).ok_or_else(|| bad(COMMAND, "key is required"))?;
    if key.len() > 256 {
        return Err(bad(COMMAND, "key exceeds 256 bytes"));
    }
    let pointer = if key.starts_with('/') { key.into() } else { format!("/{key}") };
    let defaults = serde_json::to_value(profile::platform_defaults()).map_err(|error| bad(COMMAND, error.to_string()))?;
    let mut tokens = pointer.strip_prefix('/').ok_or_else(|| bad(COMMAND, "invalid key"))?.split('/');
    let field = tokens.next().ok_or_else(|| bad(COMMAND, "invalid key"))?;
    if defaults.get(field).is_none() || tokens.clone().count() >= profile::MAX_PROFILE_DEPTH {
        return Err(bad(COMMAND, "key must address a print setting"));
    }
    for token in tokens {
        let mut chars = token.chars();
        while let Some(ch) = chars.next() {
            if ch == '~' && !matches!(chars.next(), Some('0' | '1')) {
                return Err(bad(COMMAND, "invalid JSON Pointer escape"));
            }
        }
    }
    Ok(pointer)
}

fn selected_id(s: &Session, params: &Value, level: ProfileLevel) -> Result<String> {
    let id = match params.get("id") {
        Some(value) => value.as_str().ok_or_else(|| bad(COMMAND, "id must be a string"))?.into(),
        None if level == ProfileLevel::Drawing => {
            let state = s.state()?;
            profile::drawing_key(state.path.as_deref(), &state.title)
        }
        None if matches!(level, ProfileLevel::Project | ProfileLevel::SheetSet) => return Err(bad(COMMAND, "id is required for this level")),
        None => "default".into(),
    };
    profile::store::validate_id(level, &id).map_err(|error| bad(COMMAND, error))?;
    Ok(id)
}

pub(super) fn run_profile(s: &mut Session, params: &Value) -> Result<Value> {
    let mut remaining = profile::MAX_PROFILE_BYTES.saturating_add(4096);
    validate_content(params, 0, &mut remaining).map_err(|error| bad(COMMAND, error))?;
    let action = params.get("action").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "action is required"))?;
    if matches!(action, "effective" | "explain") {
        return serde_json::to_value(s.effective_print_profile()).map_err(|error| bad(COMMAND, error.to_string()));
    }
    if !matches!(action, "get" | "set" | "unset" | "lock" | "unlock" | "import" | "export") {
        return Err(bad(COMMAND, "action must be effective, explain, get, set, unset, lock, unlock, import or export"));
    }
    let imported = if action == "import" {
        let text = params.get("text").and_then(Value::as_str).ok_or_else(|| bad(COMMAND, "text is required for import"))?;
        if text.len() > profile::MAX_PROFILE_BYTES {
            return Err(bad(COMMAND, "print profile exceeds 256 KiB"));
        }
        let value: Value = serde_json::from_str(text).map_err(|error| bad(COMMAND, error.to_string()))?;
        let mut remaining = profile::MAX_PROFILE_BYTES;
        validate_content(&value, 0, &mut remaining).map_err(|error| bad(COMMAND, error))?;
        Some(profile::migrate(value).map_err(|error| bad(COMMAND, error.to_string()))?)
    } else {
        None
    };
    let level = match params.get("level") {
        Some(value) => serde_json::from_value::<ProfileLevel>(value.clone()).map_err(|error| bad(COMMAND, error.to_string()))?,
        None => imported.as_ref().map(|doc| doc.level).unwrap_or(if matches!(action, "lock" | "unlock") {
            ProfileLevel::Company
        } else {
            ProfileLevel::User
        }),
    };
    if level == ProfileLevel::Platform && matches!(action, "get" | "export") {
        let doc = ProfileDoc {
            schema_version: profile::PRINT_PROFILE_SCHEMA,
            level,
            id: "default".into(),
            project_id: None,
            revision: 0,
            settings: serde_json::to_value(profile::platform_defaults()).map_err(|error| bad(COMMAND, error.to_string()))?,
            locks: Vec::new(),
            updated_ms: 0,
        };
        return if action == "export" {
            Ok(json!({"text": serde_json::to_string_pretty(&doc).map_err(|error| bad(COMMAND, error.to_string()))?}))
        } else {
            serde_json::to_value(doc).map_err(|error| bad(COMMAND, error.to_string()))
        };
    }
    let id = if params.get("id").is_none() {
        imported.as_ref().map(|doc| Ok(doc.id.clone())).unwrap_or_else(|| selected_id(s, params, level))?
    } else {
        selected_id(s, params, level)?
    };
    profile::store::validate_id(level, &id).map_err(|error| bad(COMMAND, error))?;
    let mut doc = s.load_print_profile_doc(level, &id).unwrap_or_else(|| ProfileDoc {
        schema_version: profile::PRINT_PROFILE_SCHEMA,
        level,
        id: id.clone(),
        project_id: None,
        revision: 0,
        settings: json!({}),
        locks: Vec::new(),
        updated_ms: 0,
    });
    if matches!(action, "get" | "export") {
        if action == "export" {
            return Ok(json!({"text": serde_json::to_string_pretty(&doc).map_err(|error| bad(COMMAND, error.to_string()))?}));
        }
        return serde_json::to_value(&doc).map_err(|error| bad(COMMAND, error.to_string()));
    }
    if matches!(action, "set" | "unset") && !matches!(level, ProfileLevel::User | ProfileLevel::Drawing | ProfileLevel::Project) {
        return Err(bad(COMMAND, "set/unset require user, drawing or project level"));
    }
    if matches!(action, "lock" | "unlock") && level != ProfileLevel::Company {
        return Err(bad(COMMAND, "lock/unlock require company level"));
    }
    let mut eff = s.effective_print_profile();
    eff.locked.retain(|_, owner| *owner < level);
    if let Some(mut imported) = imported {
        if imported.level != level || imported.id != id {
            return Err(bad(COMMAND, "import level and id must match the document"));
        }
        if let Some(fields) = imported.settings.as_object() {
            for field in fields.keys() {
                refuse_change(&eff, &format!("/{}", field.replace('~', "~0").replace('/', "~1")), COMMAND)?;
            }
        }
        // Replacing the document also removes its old fields, which must obey locks.
        if let Some(fields) = doc.settings.as_object() {
            for field in fields.keys() {
                refuse_change(&eff, &format!("/{}", field.replace('~', "~0").replace('/', "~1")), COMMAND)?;
            }
        }
        imported.revision = doc.revision;
        doc = imported;
    } else {
        let pointer = key(params)?;
        match action {
            "lock" => {
                if !doc.locks.contains(&pointer) {
                    if doc.locks.len() >= profile::MAX_PROFILE_LOCKS {
                        return Err(bad(COMMAND, "print profile exceeds 256 locks"));
                    }
                    doc.locks.push(pointer);
                }
            }
            "unlock" => doc.locks.retain(|lock| lock != &pointer),
            "set" | "unset" => {
                refuse_change(&eff, &pointer, COMMAND)?;
                let field = pointer.strip_prefix('/').ok_or_else(|| bad(COMMAND, "invalid key"))?.replace("~1", "/").replace("~0", "~");
                if doc.settings.is_null() {
                    doc.settings = json!({});
                }
                let fields = doc.settings.as_object_mut().ok_or_else(|| bad(COMMAND, "settings must be an object"))?;
                if field.contains('/') {
                    let top = pointer.strip_prefix('/').and_then(|path| path.split('/').next()).ok_or_else(|| bad(COMMAND, "invalid key"))?;
                    if !fields.contains_key(top) {
                        let settings = serde_json::to_value(&eff.settings).map_err(|error| bad(COMMAND, error.to_string()))?;
                        let value = settings.get(top).ok_or_else(|| bad(COMMAND, "unknown print setting"))?;
                        fields.insert(top.into(), value.clone());
                    }
                    if action == "unset" {
                        return Err(bad(COMMAND, "unset requires a top-level setting"));
                    }
                    let value = params.get("value").ok_or_else(|| bad(COMMAND, "value is required"))?;
                    let target = doc.settings.pointer_mut(&pointer).ok_or_else(|| bad(COMMAND, "key does not address an existing setting"))?;
                    *target = value.clone();
                } else if action == "unset" {
                    fields.remove(&field);
                } else {
                    let value = params.get("value").ok_or_else(|| bad(COMMAND, "value is required"))?;
                    fields.insert(field, value.clone());
                }
            }
            _ => return Err(bad(COMMAND, "invalid action")),
        }
    }
    doc.revision = doc.revision.checked_add(1).ok_or_else(|| bad(COMMAND, "profile revision overflow"))?;
    doc.updated_ms = CLOCK.get().map_or(0, |clock| clock());
    let value = serde_json::to_value(&doc).map_err(|error| bad(COMMAND, error.to_string()))?;
    profile::migrate(value.clone()).map_err(|error| bad(COMMAND, error.to_string()))?;
    s.write_print_profile(&doc).map_err(|error| bad(COMMAND, error))?;
    Ok(value)
}

fn refuse_change(eff: &EffectiveProfile, pointer: &str, command: &str) -> Result<()> {
    profile::check_change(eff, pointer).map_err(|error| bad(command, format!("LOCKED_BY_POLICY: {} ({})", error.pointer, error.level)))
}

pub(super) fn resolve_params(s: &mut Session, params: &Value, command: &str) -> Result<(Value, EffectiveProfile)> {
    crate::sheetplan::plan::validate_request_value(params).map_err(|error| bad(command, error))?;
    let eff = s.effective_print_profile();
    let settings = serde_json::to_value(&eff.settings).map_err(|error| bad(command, error.to_string()))?;
    if let Some(value) = params.get("plan").filter(|value| !value.is_null()) {
        let plan = crate::sheetplan::parse_plan(value).map_err(|error| bad(command, error))?;
        check_plan(s.doc()?, &plan, &eff, command)?;
    }
    let scan_mode = params.get("mode").and_then(Value::as_str).filter(|_| command == "ubplot.scan");
    let (rescan, plan_scan) = (scan_mode == Some("last"), scan_mode == Some("plan"));
    let mut result = params.clone();
    let plan_device = params.get("plan").and_then(|plan| plan.get("device")).filter(|value| !value.is_null()).cloned().or_else(|| {
        if command != "ubplot.publish" || params.get("plan").is_some() {
            return None;
        }
        let uid = s.state().ok()?.uid;
        s.ubplot.last.as_ref().filter(|last| last.doc_uid == uid).and_then(|last| last.plan.device.clone()).map(Value::String)
    });
    let fields = result.as_object_mut().ok_or_else(|| bad(command, "parameters must be an object"))?;
    if !fields.contains_key("device")
        && let Some(device) = plan_device
    {
        fields.insert("device".into(), device);
    }
    for (field, aliases) in [
        ("pages_per_file", &["pagesPerFile", "pages_per_file"][..]),
        ("naming", &["naming"][..]),
        ("device", &["device"][..]),
        ("paper", &["paper"][..]),
        ("plot_scale", &["scale", "plot_scale"][..]),
    ] {
        let default = settings.get(field).ok_or_else(|| bad(command, "missing profile setting"))?;
        for alias in aliases {
            if let Some(value) = fields.get(*alias)
                && value != default
            {
                refuse_change(&eff, &format!("/{field}"), command)?;
            }
        }
        // A `last` rescan keeps its stored request and a plan scan keeps the plan's device; Platform paper/scale defaults
        // would override what the scan detects, so only a level above Platform seeds them.
        let keeps_request = matches!(field, "device" | "paper" | "plot_scale") && rescan || field == "device" && plan_scan;
        let detected =
            matches!(field, "paper" | "plot_scale") && eff.origin.get(&format!("/{field}")).is_none_or(|level| *level == ProfileLevel::Platform);
        let seeded = !keeps_request && !detected;
        let Some(value) = aliases.iter().find_map(|alias| fields.get(*alias)).cloned().or_else(|| seeded.then(|| default.clone())) else {
            continue;
        };
        if let Some(alias) = aliases.first()
            && !value.is_null()
        {
            fields.insert((*alias).into(), value);
        }
    }
    if let Some(overrides) = fields.get("overrides").and_then(Value::as_object) {
        for value in overrides.values() {
            for (key, field) in [("paper", "paper"), ("scale", "plot_scale")] {
                if let Some(value) = value.get(key)
                    && Some(value) != settings.get(field)
                {
                    refuse_change(&eff, &format!("/{field}"), command)?;
                }
            }
        }
    }
    if let Some(value) = fields.get("naming") {
        let naming = value.as_str().ok_or_else(|| bad(command, "naming must be a string"))?;
        profile::naming_tokens(naming).map_err(|error| bad(command, error))?;
    }
    Ok((result, eff))
}

pub(super) fn profile_id(s: &mut Session) -> Result<String> {
    let mut chain = vec!["platform".to_string()];
    if s.load_print_profile_doc(ProfileLevel::Company, "default").is_some() {
        chain.push("company".into());
    }
    let state = s.state()?;
    let id = profile::drawing_key(state.path.as_deref(), &state.title);
    if let Some(doc) = s.load_print_profile_doc(ProfileLevel::Drawing, &id) {
        if let Some(id) = &doc.project_id
            && s.load_print_profile_doc(ProfileLevel::Project, id).is_some()
        {
            chain.push(format!("project:{id}"));
        }
        chain.push("drawing".into());
    }
    if s.load_print_profile_doc(ProfileLevel::User, "default").is_some() {
        chain.push("user".into());
    }
    Ok(chain.join(">"))
}

fn check_plan(drawing: &cadcraft_doc::Drawing, plan: &crate::sheetplan::SheetPlan, eff: &EffectiveProfile, command: &str) -> Result<()> {
    if let Some(device) = plan.device.as_deref()
        && device != eff.settings.device
    {
        refuse_change(eff, "/device", command)?;
    }
    let unit_mm = crate::sheetplan::plan::drawing_unit_mm(drawing).0;
    let policy_scale = eff
        .settings
        .plot_scale
        .as_ref()
        .map(|scale| serde_json::to_value(scale).map_err(|error| bad(command, error.to_string())))
        .transpose()?
        .and_then(|value| value.as_str().and_then(|text| crate::sheetplan::PlotScale::from_real_ratio(text, unit_mm)));
    for sheet in &plan.sheets {
        if let Some(paper) = &eff.settings.paper
            && sheet.paper_name.as_ref() != Some(paper)
        {
            refuse_change(eff, "/paper", command)?;
        }
        if let Some(scale) = &policy_scale
            && &sheet.plot_scale != scale
        {
            refuse_change(eff, "/plot_scale", command)?;
        }
    }
    Ok(())
}

pub(super) fn apply_plan(
    drawing: &cadcraft_doc::Drawing,
    plan: &mut crate::sheetplan::SheetPlan,
    params: &Value,
    eff: &EffectiveProfile,
    command: &str,
) -> Result<()> {
    check_plan(drawing, plan, eff, command)?;
    if command == "ubplot.scan" {
        return Ok(());
    }
    plan.device = params.get("device").and_then(Value::as_str).map(str::to_owned).or_else(|| plan.device.clone());
    let unit_mm = crate::sheetplan::plan::drawing_unit_mm(drawing).0;
    let scale = params
        .get("scale")
        .and_then(Value::as_str)
        .map(|text| crate::sheetplan::PlotScale::from_real_ratio(text, unit_mm).ok_or_else(|| bad(command, "invalid plot scale")))
        .transpose()?;
    for sheet in &mut plan.sheets {
        if let Some(scale) = &scale {
            sheet.plot_scale = scale.clone();
            sheet.scale_label = params.get("scale").and_then(Value::as_str).map(str::to_owned);
        }
        if params.get("paper").is_some() || scale.is_some() {
            let paper = params.get("paper").and_then(Value::as_str);
            crate::sheetplan::plan::resolve_sheet_paper(drawing, sheet, paper, unit_mm).map_err(|error| bad(command, error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_profile_actions_are_user_only() {
        for action in ["lock", "unlock", "import", "set"] {
            assert!(is_user_only_profile_action(&json!({"action": action, "level": "company"})));
        }
        assert!(is_user_only_profile_action(&json!({"action": "lock"})));
        assert!(!is_user_only_profile_action(&json!({"action": "effective"})));
        assert!(!is_user_only_profile_action(&json!({"action": "set", "level": "user"})));
        assert!(!is_user_only_profile_action(&json!({"action": "import", "level": "drawing"})));
        assert!(is_user_only_profile_action(&json!({"action": "import", "text": "{\"level\":\"company\"}"})));
        assert!(is_user_only_profile_action(&json!({"action": "set", "level": {"company": null}})));
        assert!(is_user_only_profile_action(&json!({"action": "set", "level": "Company"})));
        assert!(is_user_only_profile_action(&json!({"action": "import", "text": "{\"level\":{\"company\":null}}"})));
        assert!(is_user_only_profile_action(&json!({"action": "import", "text": "not json"})));
        assert!(is_user_only_profile_action(&json!({"action": "unset", "level": "company"})));
    }
}

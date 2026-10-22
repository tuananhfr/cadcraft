//! Content-based LISP approvals and the user-only permission boundary shared with MCP.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::{OnceLock, RwLock};

use serde_json::Value;

use crate::{EngineError, Result, Session};

/// Kept below the host's 64 KiB profile limit, alongside its other preferences.
pub const MAX_APPROVED: usize = 256;
pub const USER_ONLY_COMMANDS: &[&str] = &["lisptrust", "lispsandbox"];

/// Action-level denials: (command id, JSON pointer, value). Company-profile rules belong here.
pub const USER_ONLY_ACTION_RULES: &[(&str, &str, &str)] = &[];

fn store() -> &'static RwLock<BTreeSet<String>> {
    static APPROVED: OnceLock<RwLock<BTreeSet<String>>> = OnceLock::new();
    APPROVED.get_or_init(|| RwLock::new(BTreeSet::new()))
}

fn normalized(sha: &str) -> Option<String> {
    (sha.len() == 64 && sha.bytes().all(|c| c.is_ascii_hexdigit())).then(|| sha.to_ascii_lowercase())
}

pub fn source_sha(text: &str) -> String {
    crate::hash::sha256_hex(text.as_bytes())
}

pub fn approved() -> Vec<String> {
    store().read().map(|list| list.iter().cloned().collect()).unwrap_or_default()
}

pub fn set_approved(list: Vec<String>) {
    if let Ok(mut approved) = store().write() {
        *approved = list.into_iter().take(MAX_APPROVED).filter_map(|sha| normalized(&sha)).collect();
    }
}

pub fn approve(sha: &str) -> bool {
    let Some(sha) = normalized(sha) else { return false };
    let Ok(mut list) = store().write() else { return false };
    if list.contains(&sha) {
        return true;
    }
    list.len() < MAX_APPROVED && list.insert(sha)
}

pub fn revoke(sha: &str) -> bool {
    let Some(sha) = normalized(sha) else { return false };
    store().write().is_ok_and(|mut list| list.remove(&sha))
}

pub fn is_approved(sha: &str) -> bool {
    let Some(sha) = normalized(sha) else { return false };
    store().read().is_ok_and(|list| list.contains(&sha))
}

pub(crate) fn check_source(name: &str, text: &str) -> Result<()> {
    let sha = source_sha(text);
    if is_approved(&sha) { Ok(()) } else { Err(EngineError::Other(format!("LISP_NOT_REVIEWED: {name} (sha256 {sha})"))) }
}

thread_local! {
    static MCP_INPUT: Cell<bool> = const { Cell::new(false) };
}

pub fn mcp_input() -> bool {
    MCP_INPUT.get()
}

/// Scope the origin through nested commands and script tokens; restore it even after unwinding.
pub fn with_mcp_input<T>(run: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            MCP_INPUT.set(self.0);
        }
    }
    let _restore = Restore(MCP_INPUT.replace(true));
    run()
}

pub fn user_only_refusal(command: &str, params: &Value) -> Option<String> {
    let id = crate::cmd::find_command(command).map_or(command, |spec| spec.id);
    let action = params.get("action").and_then(Value::as_str).unwrap_or("list");
    let denied = USER_ONLY_COMMANDS.contains(&id) && action != "list"
        || USER_ONLY_ACTION_RULES
            .iter()
            .any(|(cmd, key, value)| *cmd == id && params.pointer(key).and_then(Value::as_str).is_some_and(|v| v.eq_ignore_ascii_case(value)));
    denied.then(|| format!("Only the user can change permissions with {id}"))
}

pub(crate) fn check_command(session: &Session, command: &str, params: &Value) -> Result<()> {
    if mcp_input() || session.lisp_running() {
        check_input(session)?;
        if let Some(message) = user_only_refusal(command, params) {
            return Err(EngineError::Other(message));
        }
    }
    Ok(())
}

pub(crate) fn check_input(session: &Session) -> Result<()> {
    if mcp_input() && session.lisp_review_pending() {
        return Err(EngineError::Other("Only the user can answer APPLOADTRUST".into()));
    }
    Ok(())
}

impl Session {
    pub fn lisp_review_pending(&self) -> bool {
        self.running.as_ref().is_some_and(|run| run.machine.name() == "APPLOADTRUST")
    }
}

//! UBREPLACE / TUBAB inputs: the box references to swap, each checked before anything is planned.

use std::collections::HashSet;

use cadcraft_doc::{EntityKind, Handle, Insert, Space};
use serde_json::{Value, json};

use super::xdata::{self, UbotTag};
use crate::cmd::{bad, str_param};
use crate::{Result, Session};

/// References per run (the core's replace limit).
pub(super) const MAX_TARGETS: usize = 100_000;

/// The fields of a box's instance tag.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct InstanceTag {
    pub instance_id: String,
    pub product_id: String,
    pub layout_id: String,
    pub count_multiplier: i16,
}

impl InstanceTag {
    pub fn with_product(&self, product_id: &str) -> UbotTag {
        UbotTag::Instance {
            instance_id: self.instance_id.clone(),
            product_id: product_id.to_string(),
            layout_id: self.layout_id.clone(),
            count_multiplier: self.count_multiplier,
        }
    }
}

/// A single, unlocked block reference in model space whose U-BOT data, if any, is a box tag.
#[derive(Debug, Clone)]
pub(super) struct Target {
    pub handle: Handle,
    pub insert: Insert,
    pub tag: Option<InstanceTag>,
}

impl Target {
    /// The reference and its tag as hashed for `expectHash`.
    pub fn hash_value(&self) -> Value {
        let i = &self.insert;
        let tag = self.tag.as_ref().map(|t| json!([t.instance_id, t.product_id, t.layout_id, t.count_multiplier]));
        json!({
            "handle": self.handle.hex(),
            "block": i.block,
            "insert": [i.insert.x, i.insert.y, i.insert.z],
            "rotation": i.rotation,
            "scale": [i.scale.x, i.scale.y, i.scale.z],
            "tag": tag,
        })
    }
}

/// `handles` (default: the selection), duplicates dropped, at most [`MAX_TARGETS`].
pub(super) fn handles(s: &Session, cmd: &str, p: &Value) -> Result<Vec<Handle>> {
    let too_many = || bad(cmd, format!("too many objects (at most {MAX_TARGETS})"));
    if p.get("handles").and_then(Value::as_array).is_some_and(|a| a.len() > MAX_TARGETS) {
        return Err(too_many());
    }
    let mut seen = HashSet::new();
    let handles: Vec<Handle> = crate::cmd::targets(s, p)?.into_iter().filter(|h| seen.insert(*h)).collect();
    if handles.len() > MAX_TARGETS {
        return Err(too_many());
    }
    Ok(handles)
}

/// `mode`: `false` for preview (the default), `true` for commit.
pub(super) fn commit_mode(cmd: &str, p: &Value) -> Result<bool> {
    match str_param(p, "mode").map(|m| m.trim().to_ascii_lowercase()).as_deref() {
        None | Some("preview") => Ok(false),
        Some("commit") => Ok(true),
        Some(other) => {
            let shown: String = other.chars().take(20).collect();
            Err(bad(cmd, format!("unknown mode '{shown}' (use preview or commit)")))
        }
    }
}

/// The CONFLICT error when the preview's `expect` hash is not the one planned now.
pub(super) fn check_expected(cmd: &str, expect: Option<&str>, now: &str) -> Result<()> {
    match expect.filter(|e| *e != now) {
        Some(expect) => {
            let shown: String = expect.chars().take(64).collect();
            Err(bad(cmd, format!("CONFLICT: inputs changed since preview (expected {shown}, now {now})")))
        }
        None => Ok(()),
    }
}

fn cells(ins: &Insert) -> u64 {
    u64::from(ins.cols.max(1)).saturating_mul(u64::from(ins.rows.max(1)))
}

/// Reads `h` as a reference that may be swapped; anything else (or a locked one) is an error naming it.
pub(super) fn target(s: &Session, cmd: &str, h: Handle) -> Result<Target> {
    let d = s.doc()?;
    let e = d.entity(h).ok_or_else(|| bad(cmd, format!("no object {}", h.hex())))?;
    if d.space_of(h) != Some(Space::Model) {
        return Err(bad(cmd, format!("{} is not in model space", h.hex())));
    }
    let EntityKind::Insert(ins) = &e.kind else { return Err(bad(cmd, format!("{} is not a block reference", h.hex()))) };
    // A tagged array stands for many boxes with one instance id; swapping it cell by cell is not possible.
    if cells(ins) > 1 {
        return Err(bad(cmd, format!("{} is a block array (MINSERT); explode it first", h.hex())));
    }
    if d.block(&ins.block).is_some_and(|b| b.xref_path.is_some()) {
        return Err(bad(cmd, format!("{} references an external drawing", h.hex())));
    }
    let tag = match xdata::read(&e.common) {
        None => None,
        Some(Ok(UbotTag::Instance { instance_id, product_id, layout_id, count_multiplier })) => {
            Some(InstanceTag { instance_id, product_id, layout_id, count_multiplier })
        }
        Some(Ok(_)) => return Err(bad(cmd, format!("{} carries U-BOT data that is not a box", h.hex()))),
        Some(Err(why)) => return Err(bad(cmd, format!("{} carries malformed U-BOT data ({why})", h.hex()))),
    };
    if crate::cmd::curves::is_locked(s, h) {
        return Err(bad(cmd, format!("{} is on a locked layer", h.hex())));
    }
    Ok(Target { handle: h, insert: ins.clone(), tag })
}

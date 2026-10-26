//! UBCHECK (spec §7): audits U-BOT boxes, slab layouts and zones in model space through the core check and reports
//! every issue; nothing is written.

use std::collections::BTreeMap;

use cadcraft_ubot::{Issue, Severity, check};
use serde_json::{Value, json};

use super::check_input::{Gathered, gather};
use super::fill_interactive::num;
use super::units::unit_scale;
use crate::cmd::{CommandSpec, bad, str_param};
use crate::{Result, Session};

const CMD: &str = "ubcheck";
/// Issues echoed one per line; the response lists them all.
const MAX_ECHOED: usize = 1000;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Check", run)
        .menu(&["Tools", "U-BOT", "Check Drawing"])
        .params(
            "{units?: mm|cm|m|in (required when INSUNITS is not in/mm/cm/m)} → {issues: [{code, severity, handle, at, message}], counts: {CODE: n}, total}; a tagged MINSERT is checked at its first cell only, so issues of its other cells go unreported",
        )
        .noundo()
}

fn code_name(i: &Issue) -> String {
    serde_json::to_value(i.code).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let unit_scale = unit_scale(s, str_param(p, "units"))?;
    let issues = {
        let catalog = &s.ubot.catalog;
        let Gathered { request, mut issues } = gather(s.doc()?, catalog, unit_scale);
        let mut all = check(&request, catalog).map_err(|e| bad(CMD, e.to_string()))?;
        all.append(&mut issues);
        // Stable, as the core orders its own: top-down, then left to right.
        all.sort_by(|a, b| b.at[1].total_cmp(&a.at[1]).then(a.at[0].total_cmp(&b.at[0])));
        all
    };
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for i in &issues {
        *counts.entry(code_name(i)).or_insert(0) += 1;
    }
    for i in issues.iter().take(MAX_ECHOED) {
        let handle = i.handle.as_deref().unwrap_or("-");
        s.echo(format!("{} {handle} at {},{}: {}", code_name(i), num(i.at[0]), num(i.at[1]), i.message));
    }
    if issues.len() > MAX_ECHOED {
        s.echo(format!("... and {} more", issues.len() - MAX_ECHOED));
    }
    let of = |severity: Severity| issues.iter().filter(|i| i.severity == severity).count();
    if issues.is_empty() {
        s.echo("UBCHECK: no issues.");
    } else {
        let (errors, warnings, info) = (of(Severity::Error), of(Severity::Warning), of(Severity::Info));
        s.echo(format!("UBCHECK: {} issues: {errors} errors, {warnings} warnings, {info} info.", issues.len()));
    }
    Ok(json!({ "issues": issues, "counts": counts, "total": issues.len() }))
}

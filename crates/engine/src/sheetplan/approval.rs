use std::collections::BTreeSet;

use super::ids::{sha256_hex, sheet_key};
use super::preflight::derive_status;
use super::{AuditEntry, MAX_AUDIT_ENTRIES, PreflightIssue, Severity, SheetPlan, SheetStatus};

pub const MAX_APPROVALS: usize = 10_000;

pub fn fingerprint(issue: &PreflightIssue, sheet_key: &str) -> String {
    sha256_hex(format!("{}|{sheet_key}|{}", issue.code, issue.message).as_bytes()).chars().take(12).collect()
}

pub(crate) fn approved(issue: &PreflightIssue, key: &str, audit: &[AuditEntry]) -> bool {
    if issue.severity != Severity::Warn || !issue.approved_by_user {
        return false;
    }
    let fingerprint = fingerprint(issue, key);
    audit.iter().any(|record| record.action == "approve" && record.sheet_key == key && record.code == issue.code && record.fingerprint == fingerprint)
}

pub(crate) fn issues(plan: &SheetPlan) -> impl Iterator<Item = (String, &PreflightIssue)> {
    plan.issues.iter().map(|issue| ("*".to_string(), issue)).chain(plan.sheets.iter().flat_map(|sheet| {
        let key = sheet_key(sheet);
        sheet.issues.iter().map(move |issue| (key.clone(), issue))
    }))
}

fn audit_keys(audit: &[AuditEntry]) -> BTreeSet<(&str, &str, &str)> {
    audit
        .iter()
        .filter(|record| record.action == "approve")
        .map(|record| (record.sheet_key.as_str(), record.code.as_str(), record.fingerprint.as_str()))
        .collect()
}

pub fn approve(plan: &mut SheetPlan, items: &[(String, String)], by: &str, at_ms: u64) -> Result<usize, String> {
    if by.chars().take(65).count() > 64 || !(by == "user" || by.strip_prefix("agent:").is_some_and(|name| !name.trim().is_empty())) {
        return Err("by must be user or agent:<name>, with 1 to 64 characters".into());
    }
    if items.len() > MAX_APPROVALS {
        return Err(format!("at most {MAX_APPROVALS} approvals are allowed"));
    }
    let remaining = MAX_AUDIT_ENTRIES.checked_sub(plan.audit.len()).ok_or_else(|| format!("audit count exceeds {MAX_AUDIT_ENTRIES}"))?;
    let targets: BTreeSet<_> = items.iter().cloned().collect();
    if let Some((key, issue)) =
        issues(plan).find(|(key, issue)| issue.severity == Severity::Block && targets.contains(&(key.clone(), issue.code.clone())))
    {
        return Err(format!("BLOCK_NOT_APPROVABLE: {key}:{}", issue.code));
    }
    let mut found = BTreeSet::new();
    let mut records = Vec::new();
    let mut fingerprints = BTreeSet::new();
    for (key, issue) in issues(plan) {
        let target = (key.clone(), issue.code.clone());
        if !targets.contains(&target) {
            continue;
        }
        if issue.severity != Severity::Warn {
            return Err(format!("only WARN issues can be approved: {key}:{}", issue.code));
        }
        found.insert(target);
        let fingerprint = fingerprint(issue, &key);
        if fingerprints.insert((key.clone(), issue.code.clone(), fingerprint.clone())) {
            if records.len() >= remaining {
                return Err(format!("audit count exceeds {MAX_AUDIT_ENTRIES}"));
            }
            records.push(AuditEntry { at_ms, by: by.into(), action: "approve".into(), sheet_key: key, code: issue.code.clone(), fingerprint });
        }
    }
    if let Some((key, code)) = targets.difference(&found).next() {
        return Err(format!("unknown sheetKey or issue code: {key}:{code}"));
    }
    // Validate the entire request before touching flags or the append-only audit.
    for entry in &mut plan.issues {
        if targets.contains(&("*".into(), entry.code.clone())) {
            entry.approved_by_user = true;
        }
    }
    for sheet in &mut plan.sheets {
        let key = sheet_key(sheet);
        for entry in &mut sheet.issues {
            if targets.contains(&(key.clone(), entry.code.clone())) {
                entry.approved_by_user = true;
            }
        }
    }
    let count = records.len();
    plan.audit.extend(records);
    refresh_status(plan);
    Ok(count)
}

pub(crate) fn validate_issue_approvals(plan: &SheetPlan) -> Result<(), String> {
    if plan.audit.len() > MAX_AUDIT_ENTRIES {
        return Err(format!("audit count exceeds {MAX_AUDIT_ENTRIES}"));
    }
    // BLOCK takes precedence even when another issue has an invalid WARN approval.
    if let Some((key, issue)) = issues(plan).find(|(_, issue)| issue.severity == Severity::Block && issue.approved_by_user) {
        return Err(format!("BLOCK_NOT_APPROVABLE: {key}:{}", issue.code));
    }
    let audit = audit_keys(&plan.audit);
    for (key, issue) in issues(plan).filter(|(_, issue)| issue.severity == Severity::Warn && issue.approved_by_user) {
        let fingerprint = fingerprint(issue, &key);
        if !audit.contains(&(key.as_str(), issue.code.as_str(), fingerprint.as_str())) {
            return Err(format!("APPROVAL_WITHOUT_AUDIT: {key}:{}", issue.code));
        }
    }
    Ok(())
}

pub fn validate_approvals(plan: &SheetPlan) -> Result<(), String> {
    validate_issue_approvals(plan)?;
    let device = plan.device.as_deref().and_then(crate::cmd::plotters::find_device);
    for sheet in &plan.sheets {
        if sheet.split.as_ref().is_some_and(|split| split.approved) && !super::split::split_approved(sheet, &plan.audit, device.as_ref()) {
            return Err(format!("APPROVAL_WITHOUT_AUDIT: {}:split", sheet_key(sheet)));
        }
    }
    Ok(())
}

pub(crate) fn merge_audit(old: &[AuditEntry], incoming: &[AuditEntry]) -> Result<Vec<AuditEntry>, String> {
    if old.len() > MAX_AUDIT_ENTRIES || incoming.len() > MAX_AUDIT_ENTRIES {
        return Err(format!("audit count exceeds {MAX_AUDIT_ENTRIES}"));
    }
    let mut seen: BTreeSet<_> = old.iter().collect();
    let added: Vec<_> = incoming.iter().filter(|record| seen.insert(*record)).collect();
    if added.len() > MAX_AUDIT_ENTRIES.saturating_sub(old.len()) {
        return Err(format!("audit count exceeds {MAX_AUDIT_ENTRIES}"));
    }
    let mut audit = old.to_vec();
    audit.extend(added.into_iter().cloned());
    Ok(audit)
}

pub fn carry_over(old: &SheetPlan, new: &mut SheetPlan) {
    let audit = audit_keys(&old.audit);
    let approved: BTreeSet<_> = issues(old)
        .filter(|(_, issue)| issue.severity == Severity::Warn && issue.approved_by_user)
        .filter_map(|(key, issue)| {
            let fingerprint = fingerprint(issue, &key);
            audit.contains(&(key.as_str(), issue.code.as_str(), fingerprint.as_str())).then_some((key, issue.code.clone(), fingerprint))
        })
        .collect();
    new.audit.clone_from(&old.audit);
    let device = new.device.as_deref().and_then(crate::cmd::plotters::find_device);
    for sheet in &mut new.sheets {
        if let Some(previous) = old.sheets.iter().find(|previous| sheet_key(previous) == sheet_key(sheet)) {
            let mut candidate = sheet.clone();
            candidate.split.clone_from(&previous.split);
            if old.device == new.device && super::split::split_approved(&candidate, &new.audit, device.as_ref()) {
                sheet.split = candidate.split;
                sheet.issues.retain(|issue| issue.code != super::codes::DEVICE_LIMIT_EXCEEDED);
            }
        }
    }
    for issue in &mut new.issues {
        issue.approved_by_user = issue.severity == Severity::Warn && approved.contains(&("*".into(), issue.code.clone(), fingerprint(issue, "*")));
    }
    for sheet in &mut new.sheets {
        let key = sheet_key(sheet);
        for issue in &mut sheet.issues {
            issue.approved_by_user =
                issue.severity == Severity::Warn && approved.contains(&(key.clone(), issue.code.clone(), fingerprint(issue, &key)));
        }
    }
    refresh_status(new);
}

pub(crate) fn refresh_status(plan: &mut SheetPlan) {
    let blocked = plan.issues.iter().any(|issue| issue.severity == Severity::Block);
    let needs_preview = plan.issues.iter().any(|issue| issue.severity == Severity::Warn && !approved(issue, "*", &plan.audit));
    for sheet in &mut plan.sheets {
        sheet.status = if blocked {
            SheetStatus::Blocked
        } else {
            match derive_status(sheet, &plan.audit) {
                SheetStatus::Ready if needs_preview => SheetStatus::NeedsPreview,
                status => status,
            }
        };
    }
}

pub fn approve_with_splits(
    plan: &mut SheetPlan,
    items: &[(String, String)],
    splits: &[(String, Option<f64>)],
    by: &str,
    at_ms: u64,
) -> Result<usize, String> {
    use super::codes;
    use super::split::{DEFAULT_OVERLAP_MM, device_limits, propose_split, split_fingerprint};

    if items.len().saturating_add(splits.len()) > MAX_APPROVALS {
        return Err(format!("at most {MAX_APPROVALS} approvals are allowed"));
    }
    if cfg!(target_arch = "wasm32") && !splits.is_empty() {
        return Err("printer splits are only available in the desktop app; choose PDF".into());
    }
    // Stage both kinds together so a bad overlap or WARN target cannot leave a partial approval.
    let mut candidate = plan.clone();
    let mut count = approve(&mut candidate, items, by, at_ms)?;
    let device = candidate.device.as_deref().and_then(crate::cmd::plotters::find_device);
    let limits = device.as_ref().and_then(device_limits);
    let mut seen = BTreeSet::new();
    for (key, overlap) in splits {
        if !seen.insert(key) {
            return Err(format!("duplicate split sheetKey: {key}"));
        }
        let sheet = candidate.sheets.iter_mut().find(|sheet| sheet_key(sheet) == *key).ok_or_else(|| format!("unknown sheetKey: {key}"))?;
        if sheet.split.is_none() {
            return Err(format!("no split proposal for {key}; choose PDF"));
        }
        let (long, short) = limits.ok_or_else(|| "the requested device is unavailable; choose PDF".to_string())?;
        let mut split = propose_split(sheet, long, short, overlap.unwrap_or(DEFAULT_OVERLAP_MM))
            .ok_or_else(|| format!("cannot split {key}; overlap must be 0..=200 mm and the short side must fit; choose PDF"))?;
        split.approved = true;
        sheet.split = Some(split);
        if candidate.audit.len() >= MAX_AUDIT_ENTRIES {
            return Err(format!("audit count exceeds {MAX_AUDIT_ENTRIES}"));
        }
        candidate.audit.push(AuditEntry {
            at_ms,
            by: by.into(),
            action: "split".into(),
            sheet_key: key.clone(),
            code: codes::DEVICE_LIMIT_EXCEEDED.into(),
            fingerprint: split_fingerprint(sheet, device.as_ref()),
        });
        sheet.issues.retain(|issue| issue.code != codes::DEVICE_LIMIT_EXCEEDED);
        count = count.saturating_add(1);
    }
    refresh_status(&mut candidate);
    *plan = candidate;
    Ok(count)
}

use std::collections::{BTreeMap, BTreeSet};

use cadcraft_engine::sheetplan::{PreflightIssue, ScanRequest, Severity, Sheet, SheetPlan, approval::fingerprint, codes, ids::sheet_key};
use serde_json::json;

use super::dialog::Action;

#[derive(Clone, Default)]
pub(super) struct State {
    show_all: bool,
    pending: Option<Confirmation>,
    split: super::split::State,
}

#[derive(Clone)]
struct Confirmation {
    code: String,
    targets: Vec<(String, String)>,
}

struct Row<'a> {
    sheet: Option<&'a Sheet>,
    issues: Vec<&'a PreflightIssue>,
}

type Audit<'a> = BTreeSet<(&'a str, &'a str, &'a str)>;

fn audit(plan: &SheetPlan) -> Audit<'_> {
    plan.audit
        .iter()
        .filter(|entry| entry.action == "approve")
        .map(|entry| (entry.sheet_key.as_str(), entry.code.as_str(), entry.fingerprint.as_str()))
        .collect()
}

fn approved(issue: &PreflightIssue, key: &str, audit: &Audit<'_>) -> bool {
    issue.severity == Severity::Warn && issue.approved_by_user && audit.contains(&(key, issue.code.as_str(), fingerprint(issue, key).as_str()))
}

pub(super) fn has_unapproved_warnings(plan: &SheetPlan) -> bool {
    let audit = audit(plan);
    // A missing profile table matters only if the chosen plot style uses it, which Publish checks itself.
    let plan_warning = |issue: &PreflightIssue| issue.severity == Severity::Warn && issue.code != codes::PLOT_PROFILE_MISSING;
    plan.issues.iter().any(|issue| plan_warning(issue) && !approved(issue, "*", &audit))
        || plan.sheets.iter().any(|sheet| {
            let key = sheet_key(sheet);
            sheet.issues.iter().any(|issue| issue.severity == Severity::Warn && !approved(issue, &key, &audit))
        })
}

pub(super) fn show(ui: &mut egui::Ui, plan: &SheetPlan, request: &ScanRequest, state: &mut State, action: &mut Option<Action>) {
    ui.checkbox(&mut state.show_all, crate::tl!("Show all sheets"));
    let audit = audit(plan);
    let mut groups: BTreeMap<(&str, Severity), BTreeMap<String, Row<'_>>> = BTreeMap::new();
    for issue in &plan.issues {
        if issue.severity >= Severity::Warn || state.show_all {
            groups
                .entry((&issue.code, issue.severity))
                .or_default()
                .entry("*".into())
                .or_insert_with(|| Row { sheet: None, issues: Vec::new() })
                .issues
                .push(issue);
        }
    }
    for sheet in &plan.sheets {
        let key = sheet_key(sheet);
        let mut visible = false;
        for issue in &sheet.issues {
            if issue.severity >= Severity::Warn || state.show_all {
                visible = true;
                groups
                    .entry((&issue.code, issue.severity))
                    .or_default()
                    .entry(key.clone())
                    .or_insert_with(|| Row { sheet: Some(sheet), issues: Vec::new() })
                    .issues
                    .push(issue);
            }
        }
        if state.show_all && !visible {
            groups.entry(("", Severity::Pass)).or_default().insert(key, Row { sheet: Some(sheet), issues: Vec::new() });
        }
    }
    for ((code, severity), rows) in groups {
        ui.push_id((code, severity as u8), |ui| {
            let sheets: Vec<_> =
                if rows.contains_key("*") { plan.sheets.iter().collect() } else { rows.values().filter_map(|row| row.sheet).collect() };
            if code.is_empty() {
                ui.strong(crate::tl!("All sheets"));
            } else {
                let severity_label = match severity {
                    Severity::Pass => crate::tl!("PASS"),
                    Severity::Info => crate::tl!("INFO"),
                    Severity::Warn => crate::tl!("WARN"),
                    Severity::Block => crate::tl!("BLOCK"),
                };
                ui.strong(crate::tf!("{code} · {n} sheets · {severity}", code = code, n = sheets.len(), severity = severity_label))
                    .on_hover_text(issue_label(code));
            }
            if severity == Severity::Warn {
                let mut targets = Vec::new();
                let mut count = 0;
                for (key, row) in &rows {
                    let mut unapproved = false;
                    for issue in &row.issues {
                        if !approved(issue, key, &audit) {
                            targets.push((key.clone(), fingerprint(issue, key)));
                            unapproved = true;
                        }
                    }
                    if unapproved {
                        count += 1;
                    }
                }
                if ui.add_enabled(count > 0, egui::Button::new(crate::tf!("Approve all ({n})", n = count))).clicked() {
                    state.pending = Some(Confirmation { code: code.into(), targets });
                }
            } else if severity == Severity::Block {
                ui.label(crate::tl!("Cannot be approved: fix the cause or exclude the sheet"));
            }
            fixes(ui, &sheets, request, action);
            for (key, row) in rows {
                ui.push_id(&key, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if let Some(sheet) = row.sheet {
                            ui.label(crate::tf!("{sheet} · {order}", sheet = sheet.sheet_id, order = sheet.order));
                            if ui.button(crate::tl!("Pick window <")).clicked() {
                                *action = Some(Action::Pick(key.clone()));
                            }
                        } else {
                            ui.label(crate::tl!("All sheets"));
                        }
                        if severity == Severity::Warn {
                            let mut checked = row.issues.iter().all(|issue| approved(issue, &key, &audit));
                            if ui.add_enabled(!checked, egui::Checkbox::new(&mut checked, crate::tl!("Approved"))).changed() && checked {
                                *action = Some(Action::Approve(json!({"approvals": [{"sheetKey": key, "code": code}], "by": "user"})));
                            }
                        }
                    });
                    if code == codes::DEVICE_LIMIT_EXCEEDED {
                        super::split::show(ui, plan, row.sheet, &mut state.split, action);
                    }
                });
            }
            ui.separator();
        });
    }
}

pub(super) fn confirmation(ctx: &egui::Context, plan: &SheetPlan, state: &mut State, action: &mut Option<Action>) {
    let Some(pending) = state.pending.clone() else { return };
    let audit = audit(plan);
    let current: BTreeSet<_> = plan
        .issues
        .iter()
        .map(|issue| ("*".to_string(), issue))
        .chain(plan.sheets.iter().flat_map(|sheet| sheet.issues.iter().map(move |issue| (sheet_key(sheet), issue))))
        .filter(|(key, issue)| issue.code == pending.code && issue.severity == Severity::Warn && !approved(issue, key, &audit))
        .map(|(key, issue)| (key.clone(), fingerprint(issue, &key)))
        .collect();
    // A stale modal must never approve a warning whose cause changed while it was open.
    if current != pending.targets.iter().cloned().collect::<BTreeSet<_>>() {
        state.pending = None;
        return;
    }
    let keys: BTreeSet<_> = pending.targets.iter().map(|(key, _)| key).collect();
    let mut close = false;
    egui::Modal::new(egui::Id::new("ubplot_approve_warnings")).show(ctx, |ui| {
        ui.set_max_width((ctx.content_rect().width() - 64.0).clamp(200.0, 440.0));
        ui.label(crate::tf!("Approve {n} warnings ({code})? This is recorded in the audit log.", n = keys.len(), code = pending.code));
        ui.horizontal(|ui| {
            if ui.button(crate::tl!("Approve")).clicked() {
                let approvals: Vec<_> = keys.iter().map(|key| json!({"sheetKey": key, "code": pending.code})).collect();
                *action = Some(Action::Approve(json!({"approvals": approvals, "by": "user"})));
                close = true;
            }
            if ui.button(crate::tl!("Cancel")).clicked() {
                close = true;
            }
        });
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
    });
    if close {
        state.pending = None;
    }
}

fn fixes(ui: &mut egui::Ui, sheets: &[&Sheet], request: &ScanRequest, action: &mut Option<Action>) {
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(!sheets.is_empty(), egui::Button::new(crate::tl!("Exclude all"))).clicked() {
            let mut next = request.clone();
            for sheet in sheets {
                let key = sheet_key(sheet);
                if !next.exclude.contains(&key) {
                    next.exclude.push(key);
                }
            }
            *action = Some(Action::Edit(Box::new(next)));
        }
        ui.menu_button(crate::tl!("Set paper for all"), |ui| {
            for (paper, label) in [
                ("A0", crate::tl!("A0")),
                ("A1", crate::tl!("A1")),
                ("A2", crate::tl!("A2")),
                ("A3", crate::tl!("A3")),
                ("A4", crate::tl!("A4")),
                ("A5", crate::tl!("A5")),
            ] {
                if ui.selectable_label(false, label).clicked() {
                    let mut next = request.clone();
                    for sheet in sheets {
                        next.overrides.entry(sheet_key(sheet)).or_default().paper = Some(paper.into());
                    }
                    *action = Some(Action::Edit(Box::new(next)));
                    ui.close();
                }
            }
        });
    });
}

pub(super) fn issue_label(code: &str) -> &str {
    match code {
        "FRAME_AMBIGUOUS" => crate::tl!("FRAME_AMBIGUOUS"),
        "FRAME_NOT_FOUND" => crate::tl!("FRAME_NOT_FOUND"),
        "FRAME_MINSERT" => crate::tl!("FRAME_MINSERT"),
        "PAPER_TOO_LARGE" => crate::tl!("PAPER_TOO_LARGE"),
        "RENDER_FAILED" => crate::tl!("RENDER_FAILED"),
        "FRAME_DUPLICATE" => crate::tl!("FRAME_DUPLICATE"),
        "FRAME_OVERLAP" => crate::tl!("FRAME_OVERLAP"),
        "FRAME_NESTED" => crate::tl!("FRAME_NESTED"),
        "FRAME_MIRRORED" => crate::tl!("FRAME_MIRRORED"),
        "FRAME_NONUNIFORM" => crate::tl!("FRAME_NONUNIFORM"),
        "XREF_MISSING" => crate::tl!("XREF_MISSING"),
        "XREF_UNLOADED" => crate::tl!("XREF_UNLOADED"),
        "SCALE_PAPER_MISMATCH" => crate::tl!("SCALE_PAPER_MISMATCH"),
        "PAPER_TOO_SMALL" => crate::tl!("PAPER_TOO_SMALL"),
        "DEVICE_LIMIT_EXCEEDED" => crate::tl!("DEVICE_LIMIT_EXCEEDED"),
        "DEVICE_LIMIT_NEAR" => crate::tl!("DEVICE_LIMIT_NEAR"),
        "DEVICE_UNAVAILABLE" => crate::tl!("DEVICE_UNAVAILABLE"),
        "FONT_MISSING" => crate::tl!("FONT_MISSING"),
        "FONT_FALLBACK" => crate::tl!("FONT_FALLBACK"),
        "SHEET_ID_DUPLICATE" => crate::tl!("SHEET_ID_DUPLICATE"),
        "REVISION_OLD" => crate::tl!("REVISION_OLD"),
        "TEMPLATE_SUPERSEDED" => crate::tl!("TEMPLATE_SUPERSEDED"),
        "KCS_LINE_DEGENERATE" => crate::tl!("KCS_LINE_DEGENERATE"),
        "KCS_NOT_MARKER" => crate::tl!("KCS_NOT_MARKER"),
        "PLOT_PROFILE_MISSING" => crate::tl!("PLOT_PROFILE_MISSING"),
        other => other,
    }
}

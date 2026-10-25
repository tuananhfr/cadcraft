//! TUBAB prompts, as LPC's TUBxy: boxes → block to replace → block to replace it with → summary → Apply.

use std::collections::BTreeMap;

use cadcraft_doc::{EntityKind, Handle};

use super::swap_input::MAX_TARGETS;
use super::tubab::{CMD, Params, commit_checked, plan, summary_line};
use crate::cmd::bad;
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const APPLY_KEYWORDS: [&str; 2] = ["Yes", "No"];

#[derive(Default)]
pub(super) struct TubabM {
    selection: SelectPhase,
    /// The block most of the picked references use: the default of `Replace block`.
    common_block: Option<String>,
    from: Option<String>,
    /// The planned swap and its hash, once `With block` is answered.
    shown: Option<(Params, String)>,
}

/// The block name used by the most picked references (first seen wins a tie); `None` when none is a reference.
fn most_common_block(s: &Session, handles: &[Handle]) -> Option<String> {
    let d = s.doc().ok()?;
    let mut counts: BTreeMap<String, (usize, usize, String)> = BTreeMap::new();
    for (order, h) in handles.iter().enumerate() {
        if let Some(EntityKind::Insert(ins)) = d.entity(*h).map(|e| &e.kind) {
            let slot = counts.entry(ins.block.to_ascii_uppercase()).or_insert((0, order, ins.block.clone()));
            slot.0 += 1;
        }
    }
    counts.into_values().max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1))).map(|(_, _, name)| name)
}

impl TubabM {
    fn selected(&mut self, s: &mut Session) -> Step {
        self.common_block = most_common_block(s, &self.selection.picked);
        if self.common_block.is_none() {
            s.echo("No block references selected.");
            return Step::Done;
        }
        Step::Continue
    }

    fn pick(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        if let Input::Pick(handles) = input
            && self.selection.picked.len().saturating_add(handles.len()) > MAX_TARGETS
        {
            return Err(bad(CMD, format!("too many objects (at most {MAX_TARGETS})")));
        }
        Ok(match self.selection.feed(s, input)? {
            SelOutcome::Empty => Step::Done,
            SelOutcome::Done(_) => self.selected(s),
            SelOutcome::More => Step::Continue,
        })
    }

    fn choose_from(&mut self, s: &mut Session, input: &Input) -> Step {
        let name = match input {
            Input::Enter => self.common_block.clone().unwrap_or_default(),
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            _ => return Step::Continue,
        };
        let picked = uses_block(s, &self.selection.picked, &name);
        if !picked {
            s.echo(format!("No selected references of block {name}."));
            return Step::Continue;
        }
        self.from = Some(name);
        Step::Continue
    }

    fn choose_to(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let name = match input {
            Input::Enter => return Ok(Step::Cancel),
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            _ => return Ok(Step::Continue),
        };
        if s.doc()?.block(&name).is_none() {
            s.echo(format!("No block named {name}."));
            return Ok(Step::Continue);
        }
        let params = Params { handles: self.selection.picked.clone(), from: self.from.clone().unwrap_or_default(), to: name };
        let plan = plan(s, &params)?;
        s.echo(summary_line(&params, &plan));
        if plan.swaps.is_empty() {
            s.echo("TUBAB: nothing to change.");
            return Ok(Step::Done);
        }
        self.shown = Some((params, plan.input_hash));
        Ok(Step::Continue)
    }

    fn answer(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let Some((params, hash)) = &self.shown else { return Ok(Step::Cancel) };
        let answer = match input {
            Input::Enter => "Yes".to_string(),
            Input::Text(t) | Input::Keyword(t) => Prompt::new("", Accept::TEXT).kw(&APPLY_KEYWORDS).match_keyword(t).unwrap_or_default(),
            _ => String::new(),
        };
        match answer.as_str() {
            "Yes" => {
                commit_checked(s, params, Some(hash), false)?;
                Ok(Step::Done)
            }
            "No" => Ok(Step::Cancel),
            _ => {
                s.echo("Answer Yes or No.");
                Ok(Step::Continue)
            }
        }
    }
}

/// Whether any picked reference uses block `name` (any case).
fn uses_block(s: &Session, handles: &[Handle], name: &str) -> bool {
    let Ok(d) = s.doc() else { return false };
    handles.iter().any(|h| matches!(d.entity(*h).map(|e| &e.kind), Some(EntityKind::Insert(ins)) if ins.block.eq_ignore_ascii_case(name)))
}

impl Interactive for TubabM {
    fn name(&self) -> &'static str {
        "TUBAB"
    }

    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.selection = SelectPhase::begin(s);
        if self.selection.done {
            return Ok(self.selected(s));
        }
        Ok(Step::Continue)
    }

    fn prompt(&self, _s: &Session) -> Prompt {
        if !self.selection.done {
            return if self.selection.removing { self.selection.prompt() } else { Prompt::new("Select boxes", Accept::SELECT) };
        }
        if self.from.is_none() {
            let p = Prompt::new("Replace block", Accept::TEXT);
            return match &self.common_block {
                Some(name) => p.default(name.clone()),
                None => p,
            };
        }
        if self.shown.is_none() {
            return Prompt::new("With block", Accept::TEXT);
        }
        Prompt::new("Apply?", Accept::TEXT).kw(&APPLY_KEYWORDS).default("Yes")
    }

    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        if matches!(input, Input::Cancel) {
            return Ok(Step::Cancel);
        }
        if !self.selection.done {
            return self.pick(s, &input);
        }
        if self.from.is_none() {
            return Ok(self.choose_from(s, &input));
        }
        if self.shown.is_none() {
            return self.choose_to(s, &input);
        }
        self.answer(s, &input)
    }
}

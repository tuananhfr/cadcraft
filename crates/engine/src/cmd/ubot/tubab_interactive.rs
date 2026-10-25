//! TUBAB prompts, as LPC's TUBxy: boxes → block to replace → block to replace it with → summary → Apply.

use cadcraft_doc::{EntityKind, Handle};

use super::swap_input::MAX_TARGETS;
use super::tubab::{CMD, Params, commit_checked, known_target, plan, summary_line};
use crate::cmd::bad;
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const APPLY_KEYWORDS: [&str; 2] = ["Yes", "No"];

#[derive(Default)]
pub(super) struct TubabM {
    selection: SelectPhase,
    from: Option<String>,
    /// The planned swap and its hash, once `With block` is answered.
    shown: Option<(Params, String)>,
}

/// The defaults of `Replace block` and `With block`: LPC's `UB-1` → `UB-2`, then the last TUBAB of this session.
fn defaults(s: &Session) -> (String, String) {
    s.ubot.tubab_last.clone().unwrap_or_else(|| ("UB-1".to_string(), "UB-2".to_string()))
}

impl TubabM {
    fn selected(&mut self, s: &mut Session) -> Step {
        if !any_reference(s, &self.selection.picked) {
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
            Input::Enter => defaults(s).0,
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
            Input::Enter => defaults(s).1,
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            _ => return Ok(Step::Continue),
        };
        if !known_target(s, &name) {
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

/// Whether any picked object is a block reference.
fn any_reference(s: &Session, handles: &[Handle]) -> bool {
    let Ok(d) = s.doc() else { return false };
    handles.iter().any(|h| matches!(d.entity(*h).map(|e| &e.kind), Some(EntityKind::Insert(_))))
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

    fn prompt(&self, s: &Session) -> Prompt {
        if !self.selection.done {
            return if self.selection.removing { self.selection.prompt() } else { Prompt::new("Select boxes", Accept::SELECT) };
        }
        let (from, to) = defaults(s);
        if self.from.is_none() {
            return Prompt::new("Replace block", Accept::TEXT).default(from);
        }
        if self.shown.is_none() {
            return Prompt::new("With block", Accept::TEXT).default(to);
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

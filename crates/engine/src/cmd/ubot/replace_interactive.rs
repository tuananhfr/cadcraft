//! UBREPLACE prompts (spec §7): boxes → (units) → product to replace with → summary → Apply.

use cadcraft_doc::Handle;

use super::fill_interactive::{MAX_KEYWORDS, UNIT_KEYWORDS, num};
use super::read::{generated_blocks, untagged_box_product};
use super::replace_cmd::{Params, commit_checked, issue_line, plan, summary_line};
use super::swap_input::{MAX_TARGETS, target};
use super::units::unit_scale;
use crate::cmd::bad;
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const CMD: &str = super::replace_cmd::CMD;
const APPLY_KEYWORDS: [&str; 2] = ["Yes", "No"];

/// The planned replacement the Apply prompt asks about.
struct Shown {
    params: Params,
    input_hash: String,
}

#[derive(Default)]
pub(super) struct ReplaceM {
    selection: SelectPhase,
    needs_units: bool,
    units: Option<String>,
    shown: Option<Shown>,
}

/// The picked objects that are U-BOT boxes UBREPLACE can swap; the others are reported and dropped.
fn boxes_only(s: &mut Session, handles: &[Handle]) -> Vec<Handle> {
    let generated = generated_blocks(&s.ubot.catalog);
    let (mut kept, mut locked, mut other) = (Vec::new(), 0usize, 0usize);
    for &h in handles {
        if crate::cmd::curves::is_locked(s, h) {
            locked += 1;
            continue;
        }
        let is_box = target(s, CMD, h).is_ok_and(|t| t.tag.is_some() || untagged_box_product(&s.ubot.catalog, &generated, &t.insert.block).is_some());
        if is_box {
            kept.push(h);
        } else {
            other += 1;
        }
    }
    if other > 0 {
        s.echo(format!("{other} object(s) ignored (not U-BOT boxes)"));
    }
    if locked > 0 {
        s.echo(format!("{locked} object(s) on a locked layer ignored"));
    }
    kept
}

impl ReplaceM {
    fn pick(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let input = match input {
            Input::Pick(handles) if !self.selection.removing && !s.select_remove_once => Input::Pick(boxes_only(s, handles)),
            other => other.clone(),
        };
        if let Input::Pick(handles) = &input
            && self.selection.picked.len().saturating_add(handles.len()) > MAX_TARGETS
        {
            return Err(bad(CMD, format!("too many objects (at most {MAX_TARGETS})")));
        }
        Ok(match self.selection.feed(s, &input)? {
            SelOutcome::Empty => Step::Done,
            SelOutcome::Done(_) => {
                self.needs_units = unit_scale(s, None).is_err();
                Step::Continue
            }
            SelOutcome::More => Step::Continue,
        })
    }

    fn choose_product(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let id = match input {
            Input::Enter => return Ok(Step::Cancel),
            Input::Text(t) | Input::Keyword(t) if t.trim() == "?" => {
                let lines: Vec<String> =
                    s.ubot.catalog.products.iter().map(|p| format!("{}  {}  {}x{} mm", p.id, p.name, num(p.width_mm), num(p.length_mm))).collect();
                for line in lines {
                    s.echo(line);
                }
                return Ok(Step::Continue);
            }
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            _ => return Ok(Step::Continue),
        };
        if s.ubot.catalog.product(&id).is_none() {
            let shown: String = id.chars().take(64).collect();
            s.echo(format!("Unknown product '{shown}' (see UBLIB)."));
            return Ok(Step::Continue);
        }
        let params = Params { handles: self.selection.picked.clone(), to: id, allow_resize: false, units: self.units.clone() };
        let plan = plan(s, &params)?;
        s.echo(summary_line(&plan));
        for w in &plan.warnings {
            s.echo(format!("UBREPLACE warning: {w}"));
        }
        if !plan.result.refused.is_empty() {
            for i in &plan.result.refused {
                s.echo(format!("UBREPLACE refused: {}", issue_line(i)));
            }
            s.echo("Nothing replaced.");
            return Ok(Step::Cancel);
        }
        if plan.swaps.is_empty() {
            s.echo("UBREPLACE: nothing to change.");
            return Ok(Step::Done);
        }
        self.shown = Some(Shown { params, input_hash: plan.input_hash });
        Ok(Step::Continue)
    }

    fn answer(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let Some(shown) = &self.shown else { return Ok(Step::Cancel) };
        let answer = match input {
            Input::Enter => "Yes".to_string(),
            Input::Text(t) | Input::Keyword(t) => Prompt::new("", Accept::TEXT).kw(&APPLY_KEYWORDS).match_keyword(t).unwrap_or_default(),
            _ => String::new(),
        };
        match answer.as_str() {
            "Yes" => {
                commit_checked(s, &shown.params, Some(&shown.input_hash), false)?;
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

impl Interactive for ReplaceM {
    fn name(&self) -> &'static str {
        "UBREPLACE"
    }

    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.selection = SelectPhase::begin(s);
        if self.selection.done {
            self.selection.picked = boxes_only(s, &self.selection.picked.clone());
            if self.selection.picked.is_empty() {
                return Ok(Step::Done);
            }
            self.needs_units = unit_scale(s, None).is_err();
        }
        Ok(Step::Continue)
    }

    fn prompt(&self, s: &Session) -> Prompt {
        if !self.selection.done {
            return if self.selection.removing { self.selection.prompt() } else { Prompt::new("Select boxes", Accept::SELECT) };
        }
        if self.needs_units && self.units.is_none() {
            return Prompt::new("Drawing units", Accept::TEXT).kw(&UNIT_KEYWORDS);
        }
        if self.shown.is_none() {
            let mut p = Prompt::new("Replace with", Accept::TEXT);
            let products = &s.ubot.catalog.products;
            if products.len() <= MAX_KEYWORDS {
                p.keywords = products.iter().map(|p| p.id.clone()).collect();
            }
            return p;
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
        if self.needs_units && self.units.is_none() {
            match &input {
                Input::Text(t) | Input::Keyword(t) if unit_scale(s, Some(t)).is_ok() => self.units = Some(t.trim().to_ascii_lowercase()),
                _ => s.echo("Enter mm, cm, m or in."),
            }
            return Ok(Step::Continue);
        }
        if self.shown.is_none() {
            return self.choose_product(s, &input);
        }
        self.answer(s, &input)
    }
}

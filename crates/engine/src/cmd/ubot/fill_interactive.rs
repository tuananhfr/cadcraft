//! UBFILL prompts (spec §7): boundary → holes and zones → (units) → box type → rib → preview → Apply.

use cadcraft_doc::{EntityKind, Handle, LwPolyline, Space};
use cadcraft_geom::{PolyVertex, Vec2};
use cadcraft_ubot::{Candidate, LayoutRecord, Status, footprint};

use super::fill_cmd::{Params, commit_checked, plan, summary_line};
use super::fill_input::{CMD, MAX_HOLES, Stored, boundary};
use super::read::{ring_of, zone_ring_of};
use super::units::unit_scale;
use crate::cmd::bad;
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::prompt::Mark;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

/// Footprints drawn by the preview and cells marked; the summary line still counts all of them.
const MAX_SHOWN: usize = 20_000;
/// Problems printed by List.
const MAX_LISTED: usize = 1_000;
/// More products than this are listed with `?` instead of as keywords.
pub(super) const MAX_KEYWORDS: usize = 12;
const DEFAULT_PRODUCT: &str = "ubot-h13-7";
pub(super) const UNIT_KEYWORDS: [&str; 4] = ["MM", "CM", "M", "IN"];
const APPLY_KEYWORDS: [&str; 3] = ["Yes", "No", "List"];

/// The previewed fill the Apply prompt asks about.
struct Shown {
    params: Params,
    input_hash: String,
    footprints: Vec<EntityKind>,
    marks: Vec<Mark>,
    problems: Vec<Candidate>,
}

#[derive(Default)]
pub(super) struct FillM {
    /// The session selection at the start; UBFILL never changes it, so Esc leaves it as it was.
    selection: Vec<Handle>,
    boundary: Option<Handle>,
    /// The boundary's own stored fill: its product and rib are the defaults (ruling R16).
    stored: Option<LayoutRecord>,
    holes: SelectPhase,
    needs_units: bool,
    units: Option<String>,
    product: Option<String>,
    shown: Option<Shown>,
}

/// Numbers in messages: up to 3 decimals, no trailing zeros.
pub(super) fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

fn outline(corners: &[Vec2]) -> EntityKind {
    let vertices = corners.iter().map(|&p| PolyVertex { p, bulge: 0.0, start_width: 0.0, end_width: 0.0 }).collect();
    EntityKind::LwPolyline(LwPolyline { vertices, closed: true, const_width: 0.0, elevation: 0.0, plinegen: false })
}

fn reason_name<T: serde::Serialize>(r: &T) -> String {
    serde_json::to_value(r).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

impl FillM {
    fn default_product(&self, s: &Session) -> Option<String> {
        let c = &s.ubot.catalog;
        let stored = self.stored.as_ref().and_then(|r| c.product(&r.product_id));
        stored.or_else(|| c.product(DEFAULT_PRODUCT)).or_else(|| c.products.first()).map(|p| p.id.clone())
    }

    /// The rib choices and the default: the gap of the stored X pitch while the product is unchanged, else the rulepack's.
    fn ribs(&self, s: &Session) -> (Vec<f64>, Option<f64>) {
        let c = &s.ubot.catalog;
        let choices = c.rulepack(None).map(|r| r.rib_mm_choices.clone()).unwrap_or_default();
        let stored = self.stored.as_ref().zip(self.product.as_deref().and_then(|id| c.product(id))).and_then(|(r, main)| {
            let rib = r.pitch_x_mm - main.width_mm;
            (r.product_id == main.id && rib.is_finite() && rib >= 0.0).then_some(rib)
        });
        (choices, stored.or_else(|| c.rulepack(None).map(|r| r.rib_mm_default)))
    }

    fn pick_boundary(&mut self, s: &mut Session, handles: &[Handle]) -> Step {
        let d = match s.doc() {
            Ok(d) => d,
            Err(_) => return Step::Cancel,
        };
        let found = handles.iter().copied().find(|h| d.space_of(*h) == Some(Space::Model) && d.entity(*h).and_then(|e| ring_of(e)).is_some());
        match found.map(|h| (h, boundary(d, h))) {
            Some((h, Ok((_, stored)))) => {
                self.boundary = Some(h);
                self.stored = match stored {
                    Stored::Own(r) => Some(r),
                    Stored::Copied(_) | Stored::None => None,
                };
            }
            Some((_, Err(_))) => s.echo("That polyline is a U-BOT zone; select the slab boundary."),
            None => s.echo("Select a closed polyline as the slab boundary."),
        }
        Step::Continue
    }

    fn pick_holes(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let input = match input {
            Input::Pick(handles) => {
                let d = s.doc()?;
                let rings: Vec<Handle> = handles
                    .iter()
                    .copied()
                    .filter(|h| {
                        Some(*h) != self.boundary && d.space_of(*h) == Some(Space::Model) && d.entity(*h).and_then(|e| zone_ring_of(e)).is_some()
                    })
                    .collect();
                let ignored = handles.len().saturating_sub(rings.len());
                if ignored > 0 {
                    s.echo(format!("{ignored} object(s) ignored (not closed polylines or circles)"));
                }
                if self.holes.picked.len().saturating_add(rings.len()) > MAX_HOLES {
                    return Err(bad(CMD, format!("too many objects (at most {MAX_HOLES})")));
                }
                Input::Pick(rings)
            }
            other => other.clone(),
        };
        let outcome = self.holes.feed(s, &input)?;
        // Picked holes are not highlighted: the selection stays the one UBFILL started with.
        s.set_selection(self.selection.clone());
        if let SelOutcome::Done(_) | SelOutcome::Empty = outcome {
            self.holes.done = true;
            self.needs_units = unit_scale(s, None).is_err();
        }
        Ok(Step::Continue)
    }

    fn choose_product(&mut self, s: &mut Session, input: &Input) -> Step {
        let id = match input {
            Input::Enter => match self.default_product(s) {
                Some(id) => id,
                None => {
                    s.echo("The U-BOT catalog has no products (see UBLIB).");
                    return Step::Cancel;
                }
            },
            Input::Text(t) | Input::Keyword(t) if t.trim() == "?" => {
                let lines: Vec<String> =
                    s.ubot.catalog.products.iter().map(|p| format!("{}  {}  {}x{} mm", p.id, p.name, num(p.width_mm), num(p.length_mm))).collect();
                for line in lines {
                    s.echo(line);
                }
                return Step::Continue;
            }
            Input::Text(t) | Input::Keyword(t) => t.trim().to_string(),
            _ => return Step::Continue,
        };
        if s.ubot.catalog.product(&id).is_none() {
            let shown: String = id.chars().take(64).collect();
            s.echo(format!("Unknown product '{shown}' (see UBLIB)."));
            return Step::Continue;
        }
        self.product = Some(id);
        Step::Continue
    }

    fn choose_rib(&mut self, s: &mut Session, input: &Input) -> Result<Step> {
        let (choices, default) = self.ribs(s);
        let is_default = |r: f64| default.is_some_and(|d| (d - r).abs() < 1e-9);
        let rib = match input {
            Input::Enter => default,
            Input::Text(t) | Input::Keyword(t) => t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0),
            _ => return Ok(Step::Continue),
        };
        let Some(rib) = rib.filter(|r| choices.is_empty() || is_default(*r) || choices.iter().any(|c| (c - r).abs() < 1e-9)) else {
            s.echo("Choose one of the listed ribs.");
            return Ok(Step::Continue);
        };
        // The default is left to the fill, so a stored pitch (which may differ per axis) is kept as it is.
        self.show(s, (!is_default(rib)).then_some(rib))?;
        Ok(Step::Continue)
    }

    fn params(&self, rib: Option<f64>) -> Params {
        Params {
            boundary: self.boundary.unwrap_or_default(),
            holes: self.holes.picked.clone(),
            product: self.product.clone().unwrap_or_default(),
            rib,
            units: self.units.clone(),
            ..Params::default()
        }
    }

    /// Plans the fill, prints the summary and keeps what the preview and marks draw.
    fn show(&mut self, s: &mut Session, rib: Option<f64>) -> Result<()> {
        let params = self.params(rib);
        let plan = plan(s, &params)?;
        s.echo(summary_line(&plan.result));
        if plan.grid_changed {
            s.echo("Grid changed: boxes will be re-laid on the new grid (manual moves are lost).");
        }
        for w in &plan.result.warnings {
            s.echo(format!("UBFILL warning: {w}"));
        }
        let us = plan.unit_scale;
        let size = s.ubot.catalog.product(&params.product).map_or(Vec2::ZERO, |p| Vec2::new(p.width_mm, p.length_mm));
        let corners = |center: [f64; 2]| -> Vec<Vec2> {
            footprint(Vec2::new(center[0], center[1]) * us, size.x, size.y, plan.record.angle_deg).corners.iter().map(|c| *c / us).collect()
        };
        let footprints = plan.result.add.iter().take(MAX_SHOWN).map(|p| outline(&corners(p.center))).collect();
        let marks = plan
            .result
            .problems
            .iter()
            .filter(|c| matches!(c.status, Status::Invalid | Status::Uncertain))
            .take(MAX_SHOWN)
            .map(|c| Mark { outline: corners(c.center), label: c.reasons.first().map(reason_name).unwrap_or_default(), warn: true })
            .collect();
        self.shown = Some(Shown { params, input_hash: plan.result.input_hash.clone(), footprints, marks, problems: plan.result.problems });
        Ok(())
    }

    fn list(s: &mut Session, problems: &[Candidate]) {
        if problems.is_empty() {
            s.echo("No problems.");
        }
        for c in problems.iter().take(MAX_LISTED) {
            let reasons: Vec<String> = c.reasons.iter().map(reason_name).collect();
            s.echo(format!("cell {},{} at {},{}: {}", c.cell[0], c.cell[1], num(c.center[0]), num(c.center[1]), reasons.join(", ")));
        }
        if problems.len() > MAX_LISTED {
            s.echo(format!("... and {} more", problems.len() - MAX_LISTED));
        }
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
            "List" => {
                Self::list(s, &shown.problems);
                Ok(Step::Continue)
            }
            _ => {
                s.echo("Answer Yes, No or List.");
                Ok(Step::Continue)
            }
        }
    }
}

impl Interactive for FillM {
    fn name(&self) -> &'static str {
        "UBFILL"
    }

    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        self.selection = s.selection();
        Ok(Step::Continue)
    }

    fn prompt(&self, s: &Session) -> Prompt {
        if self.boundary.is_none() {
            return Prompt::new("Select slab boundary", Accept::SELECT);
        }
        if !self.holes.done {
            return if self.holes.removing { self.holes.prompt() } else { Prompt::new("Select holes and zones", Accept::SELECT).default("none") };
        }
        if self.needs_units && self.units.is_none() {
            return Prompt::new("Drawing units", Accept::TEXT).kw(&UNIT_KEYWORDS);
        }
        if self.product.is_none() {
            let mut p = Prompt::new("Box type", Accept::TEXT);
            let products = &s.ubot.catalog.products;
            if products.len() <= MAX_KEYWORDS {
                p.keywords = products.iter().map(|p| p.id.clone()).collect();
            }
            return match self.default_product(s) {
                Some(id) => p.default(id),
                None => p,
            };
        }
        if self.shown.is_none() {
            let (choices, default) = self.ribs(s);
            let mut p = Prompt::new("Rib", Accept::TEXT);
            p.keywords = choices.iter().map(|c| num(*c)).collect();
            return match default {
                Some(d) => p.default(num(d)),
                None => p,
            };
        }
        Prompt::new("Apply?", Accept::TEXT).kw(&APPLY_KEYWORDS).default("Yes")
    }

    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        if matches!(input, Input::Cancel) {
            return Ok(Step::Cancel);
        }
        if self.boundary.is_none() {
            return Ok(match &input {
                Input::Pick(handles) => self.pick_boundary(s, handles),
                Input::Enter => Step::Cancel,
                _ => Step::Continue,
            });
        }
        if !self.holes.done {
            return self.pick_holes(s, &input);
        }
        if self.needs_units && self.units.is_none() {
            match &input {
                Input::Text(t) | Input::Keyword(t) if unit_scale(s, Some(t)).is_ok() => self.units = Some(t.trim().to_ascii_lowercase()),
                _ => s.echo("Enter mm, cm, m or in."),
            }
            return Ok(Step::Continue);
        }
        if self.product.is_none() {
            return Ok(self.choose_product(s, &input));
        }
        if self.shown.is_none() {
            return self.choose_rib(s, &input);
        }
        self.answer(s, &input)
    }

    fn preview(&self, _s: &Session, _cursor: Vec2) -> Vec<EntityKind> {
        self.shown.as_ref().map_or_else(Vec::new, |shown| shown.footprints.clone())
    }

    fn marks(&self, _s: &Session) -> Vec<Mark> {
        self.shown.as_ref().map_or_else(Vec::new, |shown| shown.marks.clone())
    }
}

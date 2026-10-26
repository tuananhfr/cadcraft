use std::collections::BTreeSet;

use cadcraft_doc::{EntityKind, Handle, entity_bounds};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{bad, run_scan};
use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::prompt::Mark;
use crate::sheetplan::{MAX_SCAN_FRAMES, ScanMode, ScanRequest, Severity, SheetStatus, detect_frames, is_frame_candidate};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const MAX_INPUT_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Default)]
enum Mode {
    #[default]
    Choose,
    Block,
    Window,
    Layout,
}

#[derive(Default)]
pub(super) struct UbplotM {
    mode: Mode,
    select: SelectPhase,
    windows: Vec<Value>,
    first_corner: Option<Vec2>,
    kcs: bool,
}

impl UbplotM {
    /// KCS Plotter's KP: scans every marker line as soon as it starts.
    pub(super) fn kcs() -> Self {
        Self { kcs: true, ..Self::default() }
    }

    fn finish(&self, s: &mut Session, request: ScanRequest) -> Result<Step> {
        if match request.mode {
            ScanMode::Block => request.handles.is_empty(),
            ScanMode::Window => request.windows.is_empty(),
            ScanMode::Layout => request.layouts.is_empty(),
            _ => false,
        } {
            s.echo("No frames selected");
            return Ok(Step::Cancel);
        }
        let params = serde_json::to_value(request).map_err(|error| bad("ubplot", error.to_string()))?;
        run_scan(s, &params)?;
        let last = s.ubplot.last.as_ref().ok_or_else(|| bad("ubplot", "no scanned plan"))?;
        let n = last.plan.sheets.len();
        let b = last.plan.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Blocked).count();
        let w = last
            .plan
            .issues
            .iter()
            .chain(last.plan.sheets.iter().flat_map(|sheet| &sheet.issues))
            .filter(|issue| issue.severity == Severity::Warn)
            .count();
        s.ubplot.review = true;
        s.echo(format!("UBPLOT: {n} sheet(s), {b} blocked, {w} warning(s)"));
        Ok(Step::Done)
    }

    fn pick(&mut self, s: &mut Session, handles: Vec<Handle>) -> Result<Step> {
        if handles.len() > MAX_SCAN_FRAMES {
            return Err(bad("ubplot", format!("selection exceeds {MAX_SCAN_FRAMES} entries")));
        }
        let d = s.doc()?;
        let frames: Vec<_> = handles.iter().copied().filter(|h| is_frame_candidate(d, *h, &s.ubplot.registry)).collect();
        let m = handles.len().saturating_sub(frames.len());
        let removing = self.select.removing || s.select_remove_once;
        if !removing {
            let extra = frames.iter().filter(|h| !self.select.picked.contains(h)).collect::<BTreeSet<_>>().len();
            if self.select.picked.len().saturating_add(extra) > MAX_SCAN_FRAMES {
                return Err(bad("ubplot", format!("selection exceeds {MAX_SCAN_FRAMES} frames")));
            }
        }
        let before = self.select.picked.len();
        self.select.feed(s, &Input::Pick(frames))?;
        let t = self.select.picked.len();
        let n = t.saturating_sub(before);
        s.echo(format!("{n} frame(s) added, {m} ignored (not frames), {t} total"));
        Ok(Step::Continue)
    }
}

impl Interactive for UbplotM {
    fn name(&self) -> &'static str {
        "UBPLOT"
    }

    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if self.kcs {
            return self.finish(s, ScanRequest { mode: ScanMode::Kcs, ..Default::default() });
        }
        if s.settings.pickfirst {
            let selection = s.selection();
            if selection.len() > MAX_SCAN_FRAMES {
                return Err(bad("ubplot", format!("selection exceeds {MAX_SCAN_FRAMES} entries")));
            }
            if selection.iter().any(|h| s.doc().is_ok_and(|d| is_frame_candidate(d, *h, &s.ubplot.registry))) {
                self.mode = Mode::Block;
                return self.pick(s, selection);
            }
        }
        Ok(Step::Continue)
    }

    fn prompt(&self, _s: &Session) -> Prompt {
        match self.mode {
            Mode::Choose => {
                // Keep the specified wording while retaining keywords for typed shortcut matching.
                let mut prompt = Prompt::new("Plot frames by [Block/Window/Layout/Last/Kcs] <Block>:", Accept::TEXT)
                    .kw(&["Block", "Window", "Layout", "Last", "Kcs"])
                    .default("Block");
                prompt.raw = true;
                prompt
            }
            Mode::Block => Prompt::new("Select frame blocks", Accept::SELECT),
            Mode::Window => match self.first_corner {
                Some(first) => Prompt::new("Specify opposite corner", Accept::POINT).corner(first),
                None => Prompt::new("Specify first corner of sheet window", Accept::POINT),
            },
            Mode::Layout => Prompt::new("Enter layout names (comma-separated) or *", Accept::TEXT),
        }
    }

    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        if input == Input::Cancel {
            return Ok(Step::Cancel);
        }
        if matches!(&input, Input::Text(text) | Input::Keyword(text) if text.len() > MAX_INPUT_TEXT_BYTES) {
            return Err(bad("ubplot", "input text exceeds 64 MiB"));
        }
        match self.mode {
            Mode::Choose => {
                let choice = match input {
                    Input::Enter => "block".into(),
                    Input::Keyword(text) | Input::Text(text) => text.trim().to_ascii_lowercase(),
                    _ => return Err(bad("ubplot", "choose Block, Window, Layout, Last or Kcs")),
                };
                self.mode = match choice.as_str() {
                    "b" | "block" => Mode::Block,
                    "w" | "window" => Mode::Window,
                    "l" | "layout" => Mode::Layout,
                    "last" => return self.finish(s, ScanRequest { mode: ScanMode::Last, ..Default::default() }),
                    "k" | "kcs" => return self.finish(s, ScanRequest { mode: ScanMode::Kcs, ..Default::default() }),
                    _ => return Err(bad("ubplot", "choose Block, Window, Layout, Last or Kcs")),
                };
                Ok(Step::Continue)
            }
            Mode::Block => {
                if let Input::Pick(handles) = input {
                    return self.pick(s, handles);
                }
                match self.select.feed(s, &input)? {
                    SelOutcome::Done(handles) => {
                        self.finish(s, ScanRequest { handles: handles.into_iter().map(Handle::hex).collect(), ..Default::default() })
                    }
                    SelOutcome::Empty => {
                        s.echo("No frames selected");
                        Ok(Step::Cancel)
                    }
                    SelOutcome::More => Ok(Step::Continue),
                }
            }
            Mode::Window => match input {
                Input::Point(point) if point.is_finite() => {
                    if let Some(first) = self.first_corner {
                        if self.windows.len() >= MAX_SCAN_FRAMES {
                            return Err(bad("ubplot", format!("selection exceeds {MAX_SCAN_FRAMES} windows")));
                        }
                        if point.x == first.x || point.y == first.y {
                            return Err(bad("ubplot", "sheet window must have positive width and height"));
                        }
                        self.windows.push(json!([[first.x, first.y], [point.x, point.y]]));
                        self.first_corner = None;
                    } else {
                        self.first_corner = Some(point);
                    }
                    Ok(Step::Continue)
                }
                Input::Enter if self.first_corner.is_none() => {
                    self.finish(s, ScanRequest { mode: ScanMode::Window, windows: self.windows.clone(), ..Default::default() })
                }
                _ => Err(bad("ubplot", "specify a finite window corner")),
            },
            Mode::Layout => {
                let layouts = match input {
                    Input::Text(text) | Input::Keyword(text) => text
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                        .take(MAX_SCAN_FRAMES + 1)
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                    Input::Enter => Vec::new(),
                    _ => return Err(bad("ubplot", "enter layout names or *")),
                };
                if layouts.len() > MAX_SCAN_FRAMES {
                    return Err(bad("ubplot", format!("selection exceeds {MAX_SCAN_FRAMES} layouts")));
                }
                self.finish(s, ScanRequest { mode: ScanMode::Layout, layouts, ..Default::default() })
            }
        }
    }

    fn marks(&self, s: &Session) -> Vec<Mark> {
        let Ok(d) = s.doc() else { return Vec::new() };
        detect_frames(d, &self.select.picked, &s.ubplot.registry)
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| {
                let warn = candidate.outline.is_none() || candidate.issues.iter().any(|issue| issue.severity >= Severity::Warn);
                let outline = match candidate.outline {
                    Some(outline) => outline.map(|[x, y]| Vec2::new(x, y)).to_vec(),
                    None => d
                        .entity(candidate.insert)
                        .filter(|entity| matches!(entity.kind, EntityKind::Insert(_)))
                        .map(|entity| entity_bounds(d, entity, 0))
                        .filter(|bounds| !bounds.is_empty() && bounds.min.is_finite() && bounds.max.is_finite())
                        .map(|bounds| bounds.corners().to_vec())
                        .unwrap_or_default(),
                };
                Mark { outline, label: (index + 1).to_string(), warn }
            })
            .collect()
    }
}

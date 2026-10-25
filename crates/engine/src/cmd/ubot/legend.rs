//! UBLEGEND (spec §7, §14.3): the UBCOUNT result as a TABLE in the LPC sheet layout, built with TABLE's own
//! constructor in the current space and table style.

use cadcraft_doc::{EntityKind, Table, TableStyle};
use cadcraft_geom::Vec2;
use cadcraft_ubot::Counts;
use serde_json::{Value, json};

use super::count_cmd::{Scope, counts, loose_unit_scale};
use crate::cmd::table::make_table;
use crate::cmd::{CommandSpec, bad, point_req, size_param};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const CMD: &str = "ublegend";
/// Text height on LPC sheets, in mm.
const TEXT_MM: f64 = 250.0;
/// Larger text heights would make the table's sizes overflow.
const MAX_TEXT_HEIGHT: f64 = 1e9;
/// TABLE's own row cap, less the title and header rows.
const MAX_ROWS: usize = 1998;
// Drawing content in the LPC sheet convention, written verbatim (not interface text).
const TITLE: &str = "THỐNG KÊ HỘP UBOT";
const HEAD_TYPE: &str = "Loại hộp";
const HEAD_COUNT: &str = "Số lượng (hộp)";
const TOTAL: &str = "Tổng";

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Legend", run)
        .menu(&["Tools", "U-BOT", "Box Legend"])
        .params(
            "{at: [x,y] (top-left corner), textHeight? (drawing units; default 250 mm), boundary?: handle (count only inside this slab), units?: mm|cm|m|in} → {handle, rows, total}",
        )
        .interactive(|_| Ok(Box::new(LegendM)))
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let at = point_req(CMD, p, "at")?;
    let text_height = size_param(CMD, p, "textHeight", false)?;
    if text_height.is_some_and(|h| h > MAX_TEXT_HEIGHT) {
        return Err(bad(CMD, format!("`textHeight` must be at most {MAX_TEXT_HEIGHT}")));
    }
    insert(s, at, text_height, &Scope::parse(CMD, p)?)
}

fn nothing_to_count(c: &Counts) -> Result<()> {
    if c.total == 0 { Err(bad(CMD, "no U-BOT boxes to count")) } else { Ok(()) }
}

fn insert(s: &mut Session, at: Vec2, text_height: Option<f64>, scope: &Scope) -> Result<Value> {
    let c = counts(s, CMD, scope)?;
    nothing_to_count(&c)?;
    if c.rows.len() > MAX_ROWS {
        return Err(bad(CMD, format!("{} box types are too many for one table (at most {MAX_ROWS})", c.rows.len())));
    }
    let th = match text_height {
        Some(h) => h,
        None => TEXT_MM / loose_unit_scale(s, scope.units.as_deref())?,
    };
    let table = legend(&current_style(s)?, at, th, &c);
    let h = s.add_entity(EntityKind::Table(table))?;
    let rows = c.rows.len();
    s.echo(format!("UBLEGEND: table of {rows} box types, {} boxes.", c.total));
    Ok(json!({ "handle": h.hex(), "rows": rows, "total": c.total }))
}

/// The drawing's current table style (CTABLESTYLE, else the first one).
fn current_style(s: &Session) -> Result<TableStyle> {
    let d = s.doc()?;
    let cur = d.header.str("CTABLESTYLE", "Standard");
    Ok(d.table_styles.iter().find(|t| t.name.eq_ignore_ascii_case(&cur)).or(d.table_styles.first()).cloned().unwrap_or_default())
}

/// Title, header, a row per counted box type and the total; text `th` high, the style's margins scaled with it.
fn legend(style: &TableStyle, at: Vec2, th: f64, c: &Counts) -> Table {
    let k = th / style.text_height;
    let mut st = style.clone();
    st.margin = if k.is_finite() && k > 0.0 { style.margin * k } else { th / 3.0 };
    st.text_height = th;
    let mut cells: Vec<Vec<String>> = c.rows.iter().map(|r| vec![r.name.clone(), r.total.to_string()]).collect();
    cells.push(vec![TOTAL.to_string(), c.total.to_string()]);
    let header = vec![HEAD_TYPE.to_string(), HEAD_COUNT.to_string()];
    let mut t = make_table(&st, at, cells.len(), 2, None, None, Some(TITLE), Some(header.clone()), &cells);
    // Columns as wide as their longest text, then widened together until the larger title text fits.
    let chars = |s: &str| s.chars().count().clamp(1, 80) as f64;
    for (col, w) in t.col_widths.iter_mut().enumerate() {
        let widest = header.iter().chain(cells.iter().flatten()).skip(col).step_by(2).map(|s| chars(s)).fold(1.0, f64::max);
        *w = widest * th * 0.9 + 2.0 * st.margin + th;
    }
    let title_w = chars(TITLE) * th * 1.4 * 0.9 + 2.0 * st.margin + th;
    let sum: f64 = t.col_widths.iter().sum();
    if sum > 0.0 && sum < title_w {
        t.col_widths.iter_mut().for_each(|w| *w *= title_w / sum);
    }
    t
}

/// UBLEGEND at the command line: one insertion point.
struct LegendM;

impl Interactive for LegendM {
    fn name(&self) -> &'static str {
        "UBLEGEND"
    }

    fn prompt(&self, _s: &Session) -> Prompt {
        Prompt::new("Insertion point", Accept::POINT)
    }

    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        match i {
            Input::Point(at) => {
                insert(s, at, None, &Scope::default())?;
                Ok(Step::Done)
            }
            Input::Enter => Ok(Step::Cancel),
            _ => Ok(Step::Continue),
        }
    }
}

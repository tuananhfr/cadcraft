//! KCS Plotter's commands: KP plots every marker line, KL draws one.

use cadcraft_doc::{EntityKind, Layer, Space};
use cadcraft_geom::Vec2;
use serde_json::{Value, json};

use super::{bad, run_scan};
use crate::cmd::helpers::line;
use crate::cmd::point_req;
use crate::sheetplan::kcs::KCS_LAYER;
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

pub(super) fn run_kcsplotter(s: &mut Session, params: &Value) -> Result<Value> {
    let mut params = match params {
        Value::Null => json!({}),
        Value::Object(_) => params.clone(),
        _ => return Err(bad("kcsplotter", "parameters must be an object")),
    };
    let object = params.as_object_mut().ok_or_else(|| bad("kcsplotter", "parameters must be an object"))?;
    if object.get("mode").is_some_and(|mode| mode.as_str() != Some("kcs")) {
        return Err(bad("kcsplotter", "mode must be kcs"));
    }
    object.insert("mode".into(), json!("kcs"));
    run_scan(s, &params)
}

pub(super) fn run_kcsarea(s: &mut Session, params: &Value) -> Result<Value> {
    let (from, to) = (point_req("kcsarea", params, "from")?, point_req("kcsarea", params, "to")?);
    add_marker(s, from, to)
}

fn add_marker(s: &mut Session, from: Vec2, to: Vec2) -> Result<Value> {
    if from.x == to.x || from.y == to.y {
        return Err(bad("kcsarea", "the plot area needs a width and a height"));
    }
    let d = s.doc_mut()?;
    let layer = match d.layer(KCS_LAYER) {
        Some(layer) => layer.name.clone(),
        None => {
            // Markers must never print, so a new marker layer starts with plotting off.
            d.layers.push(Layer { plot: false, ..Layer::new(KCS_LAYER) });
            KCS_LAYER.to_string()
        }
    };
    let mut common = s.current_common();
    common.layer = layer;
    let handle = s.doc_mut()?.add(&Space::Model, common, line(from, to))?;
    Ok(json!({"handle": handle.hex()}))
}

/// KL: the first and last point of one plot area's diagonal.
#[derive(Default)]
pub(super) struct KcsAreaM {
    first: Option<Vec2>,
}

impl Interactive for KcsAreaM {
    fn name(&self) -> &'static str {
        "KCSAREA"
    }

    fn prompt(&self, _s: &Session) -> Prompt {
        match self.first {
            None => Prompt::new("Specify first point of the plot area", Accept::POINT),
            Some(first) => Prompt::new("Specify last point of the plot area", Accept::POINT).base(first),
        }
    }

    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        match (input, self.first) {
            (Input::Point(point), None) if point.is_finite() => {
                self.first = Some(point);
                Ok(Step::Continue)
            }
            (Input::Point(point), Some(first)) if point.is_finite() => {
                add_marker(s, first, point)?;
                Ok(Step::Done)
            }
            (Input::Cancel, _) => Ok(Step::Cancel),
            _ => Ok(Step::Continue),
        }
    }

    fn preview(&self, _s: &Session, cursor: Vec2) -> Vec<EntityKind> {
        let Some(first) = self.first else { return Vec::new() };
        let corners = [first, Vec2::new(cursor.x, first.y), cursor, Vec2::new(first.x, cursor.y), first];
        let mut preview: Vec<_> = corners
            .windows(2)
            .filter_map(|pair| match pair {
                [a, b] => Some(line(*a, *b)),
                _ => None,
            })
            .collect();
        preview.push(line(first, cursor));
        preview
    }
}

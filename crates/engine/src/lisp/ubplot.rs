use cadcraft_doc::{Entity, EntityKind, dynblock};
use serde_json::{Value as Json, json};

use super::LispError;
use super::builtins::args_err;
use super::eval::{Ctx, Flow};
use super::value::{Value, charge_bytes, check_cells, check_string_len};
use crate::sheetplan::{GeometryRule, MAX_SCAN_FRAMES, MAX_TEMPLATES, TemplateStatus};

type Result<T> = std::result::Result<T, LispError>;

fn arity(args: &[Value], min: usize, max: usize) -> Result<()> {
    if args.len() < min {
        return Err(args_err(true));
    }
    if args.len() > max {
        return Err(args_err(false));
    }
    Ok(())
}

fn argument(args: &[Value], index: usize) -> Result<&Value> {
    args.get(index).ok_or_else(|| args_err(true))
}

fn string(value: &Value) -> Result<&str> {
    match value {
        Value::Str(text) => Ok(text),
        other => Err(LispError::bad_arg("stringp", other)),
    }
}

fn text(value: &str) -> Result<Value> {
    check_string_len(value.len())?;
    charge_bytes(value.len())?;
    Ok(Value::str(value))
}

fn entity<'a>(ctx: &'a Ctx<'_>, value: &Value) -> Result<&'a Entity> {
    let Value::Ename(handle) = value else { return Err(LispError::bad_arg("lentityp", value)) };
    ctx.s
        .doc()
        .map_err(|error| LispError::new(error.to_string()))?
        .entity(*handle)
        .map(AsRef::as_ref)
        .ok_or_else(|| LispError::bad_arg("lentityp", value))
}

pub(super) fn getpropertyvalue(ctx: &mut Ctx<'_>, args: Vec<Value>) -> Result<Flow> {
    arity(&args, 2, 2)?;
    let entity = entity(ctx, argument(&args, 0)?)?;
    let property = string(argument(&args, 1)?)?.to_ascii_uppercase();
    let missing = || LispError::new(format!("bad argument value: property {property}"));
    let value = match property.as_str() {
        "LAYER" => text(&entity.common.layer)?,
        "HANDLE" => text(&entity.handle.hex())?,
        _ => {
            let EntityKind::Insert(insert) = &entity.kind else { return Err(missing()) };
            match property.as_str() {
                "EFFECTIVENAME" => {
                    let drawing = ctx.s.doc().map_err(|error| LispError::new(error.to_string()))?;
                    text(&dynblock::effective_name(drawing, entity.handle).ok_or_else(missing)?)?
                }
                "BLOCKNAME" => text(&insert.block)?,
                "ROTATION" => Value::Real(insert.rotation),
                "POSITION" => Value::point(&[insert.insert.x, insert.insert.y, insert.insert.z]),
                _ => return Err(missing()),
            }
        }
    };
    Ok(Flow::Value(value))
}

pub(super) fn attribs(ctx: &mut Ctx<'_>, args: Vec<Value>) -> Result<Flow> {
    arity(&args, 1, 1)?;
    let entity = entity(ctx, argument(&args, 0)?)?;
    let EntityKind::Insert(insert) = &entity.kind else { return Ok(Flow::Value(Value::Nil)) };
    if insert.attribs.len() > MAX_SCAN_FRAMES {
        return Err(LispError::new("too many INSERT attributes"));
    }
    let mut values = Vec::with_capacity(insert.attribs.len());
    for attribute in &insert.attribs {
        values.push(Value::cons(text(&attribute.tag)?, text(&attribute.text.value)?));
        check_cells()?;
    }
    Ok(Flow::Value(Value::list(values)))
}

fn idle(ctx: &Ctx<'_>) -> Result<()> {
    if ctx.s.running.is_some() {
        return Err(LispError::new("ubplot-scan can't run while a command is active"));
    }
    Ok(())
}

fn parse_json(value: &Value) -> Result<Json> {
    let source = string(value)?;
    check_string_len(source.len())?;
    serde_json::from_str(source).map_err(|error| LispError::new(error.to_string()))
}

fn execute(ctx: &mut Ctx<'_>, command: &str, params: Json) -> Result<Flow> {
    let result = ctx.s.execute(command, &params).map_err(|error| LispError::new(error.to_string()))?;
    let result = serde_json::to_string(&result).map_err(|error| LispError::new(error.to_string()))?;
    check_string_len(result.len())?;
    Ok(Flow::Value(Value::str(&result)))
}

pub(super) fn scan(ctx: &mut Ctx<'_>, args: Vec<Value>) -> Result<Flow> {
    arity(&args, 1, 2)?;
    idle(ctx)?;
    let value = argument(&args, 0)?;
    let Value::PickSet(selection) = value else { return Err(LispError::bad_arg("lselsetp", value)) };
    let handles = {
        let items = selection.items.lock().map_err(|_| LispError::new("could not read selection set"))?;
        if items.len() > MAX_SCAN_FRAMES {
            return Err(LispError::new(format!("handles exceeds {MAX_SCAN_FRAMES} entries")));
        }
        items.iter().map(|handle| handle.hex()).collect::<Vec<_>>()
    };
    let mut params = match args.get(1).filter(|value| !value.is_nil()) {
        Some(value) => match parse_json(value)? {
            Json::Object(settings) => settings,
            _ => return Err(LispError::new("bad argument value: settings must be a JSON object")),
        },
        None => serde_json::Map::new(),
    };
    params.insert("mode".into(), json!("block"));
    params.insert("handles".into(), json!(handles));
    execute(ctx, "ubplot.scan", Json::Object(params))
}

fn publish_path(_ctx: &mut Ctx<'_>, path: Option<&Value>) -> Result<Option<String>> {
    let path = path.filter(|value| !value.is_nil()).map(|value| string(value).map(str::to_string)).transpose()?;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = &path {
        super::sandbox::check_write(path).map_err(LispError::new)?;
        let extension = std::path::Path::new(path).extension().and_then(|extension| extension.to_str());
        if !extension.is_some_and(|extension| extension.eq_ignore_ascii_case("pdf")) {
            return Err(LispError::new(super::sandbox::blocked(path)));
        }
    }
    Ok(path)
}

pub(super) fn publish(ctx: &mut Ctx<'_>, args: Vec<Value>) -> Result<Flow> {
    arity(&args, 1, 2)?;
    idle(ctx)?;
    let plan = parse_json(argument(&args, 0)?)?;
    let path = publish_path(ctx, args.get(1))?;
    let mut params = json!({"plan": plan, "background": false});
    if let Some(path) = path {
        let Some(params) = params.as_object_mut() else { return Err(LispError::new("invalid publish parameters")) };
        params.insert("path".into(), json!(path));
    }
    execute(ctx, "ubplot.publish", params)
}

fn strings(key: &str, values: &[String]) -> Result<Value> {
    if values.len() > MAX_SCAN_FRAMES {
        return Err(LispError::new("too many template values"));
    }
    let mut result = vec![text(key)?];
    for value in values {
        result.push(text(value)?);
    }
    Ok(Value::list(result))
}

pub(super) fn templates(ctx: &mut Ctx<'_>, args: Vec<Value>) -> Result<Flow> {
    arity(&args, 0, 0)?;
    let registry = &ctx.s.ubplot.registry;
    if registry.templates.len() > MAX_TEMPLATES {
        return Err(LispError::new("too many frame templates"));
    }
    let mut result = Vec::new();
    for template in &registry.templates {
        if template.status != TemplateStatus::Approved {
            continue;
        }
        if template.geometry.len() > MAX_SCAN_FRAMES {
            return Err(LispError::new("too many template geometry rules"));
        }
        let mut frame = vec![text("frame")?];
        if let Some((min, max)) = template.geometry.iter().find_map(|rule| match rule {
            GeometryRule::LocalRect { min, max } => Some((*min, *max)),
            _ => None,
        }) {
            let [x0, y0] = min;
            let [x1, y1] = max;
            frame.extend([[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|point| Value::point(&point)));
        }
        let mut paper = vec![text("paper")?];
        if let Some([width, height]) = template.media_mm.first() {
            paper.extend([Value::Real(*width), Value::Real(*height), if width >= height { Value::t() } else { Value::Nil }]);
        }
        result.push(Value::list(vec![
            text(&template.versioned_id())?,
            strings("names", &template.block_names)?,
            Value::list(frame),
            Value::list(paper),
            strings("sheet_id_tags", &template.attributes.sheet_id)?,
        ]));
        check_cells()?;
    }
    Ok(Flow::Value(Value::list(result)))
}

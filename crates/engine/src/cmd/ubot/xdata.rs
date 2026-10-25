//! The `LPC_UBOT` xdata record (spec §4.1): fixed field order, read by position, never guessed.

use cadcraft_doc::{Common, RawTag};
use cadcraft_ubot::{LayoutRecord, ZoneType};

pub const APP: &str = "LPC_UBOT";
pub const VERSION: &str = "ubot/1";
/// Longest `1000` string a DXF/DWG xdata group holds; ids and hashes are refused, never cut.
pub const MAX_STRING_BYTES: usize = 255;
/// `1040 -1` on a zone means "the rulepack's clearance".
const RULEPACK_CLEARANCE: f64 = -1.0;
/// File values quoted in messages are cut to this many characters.
const SHOWN_CHARS: usize = 20;

#[derive(Debug, Clone, PartialEq)]
pub enum UbotTag {
    Instance { instance_id: String, product_id: String, layout_id: String, count_multiplier: i16 },
    Layout { record: LayoutRecord, boundary_handle: String },
    Zone { zone_id: String, zone_type: ZoneType, clearance_mm: Option<f64>, alt_product_id: Option<String> },
}

/// `None` when the object has no `LPC_UBOT` data; `Err` when the record is malformed.
pub fn read(common: &Common) -> Option<Result<UbotTag, String>> {
    let range = block_range(&common.xdata)?;
    let fields = common.xdata.get(range.start + 1..range.end).unwrap_or(&[]);
    Some(parse(fields))
}

/// Replaces the `LPC_UBOT` record (or appends one); other applications' data keeps its place and order.
/// A string over 255 bytes or a revision outside the 32-bit group is refused and nothing changes.
/// Numbers must be finite: a non-finite value is written as is and reads back as invalid.
pub fn write(common: &mut Common, tag: &UbotTag) -> Result<(), String> {
    let mut tags = vec![RawTag { code: 1001, value: APP.into() }];
    tags.extend(fields(tag)?);
    match block_range(&common.xdata) {
        Some(range) => {
            let at = range.start;
            common.xdata.splice(range, tags);
            // A hostile file may carry the application twice; keep only the record just written.
            while let Some(extra) = block_range_from(&common.xdata, at + 1) {
                common.xdata.drain(extra);
            }
        }
        None => common.xdata.extend(tags),
    }
    Ok(())
}

/// Removes every `LPC_UBOT` record.
pub fn clear(common: &mut Common) {
    while let Some(range) = block_range(&common.xdata) {
        common.xdata.drain(range);
    }
}

fn block_range(xdata: &[RawTag]) -> Option<std::ops::Range<usize>> {
    block_range_from(xdata, 0)
}

fn block_range_from(xdata: &[RawTag], from: usize) -> Option<std::ops::Range<usize>> {
    let start = xdata.iter().enumerate().skip(from).find(|(_, t)| t.code == 1001 && t.value.trim().eq_ignore_ascii_case(APP)).map(|(i, _)| i)?;
    let end = xdata.iter().enumerate().skip(start + 1).find(|(_, t)| t.code == 1001).map_or(xdata.len(), |(i, _)| i);
    Some(start..end)
}

fn shown(v: &str) -> String {
    v.chars().take(SHOWN_CHARS).collect()
}

pub fn zone_type_name(t: ZoneType) -> &'static str {
    match t {
        ZoneType::Exclusion => "exclusion",
        ZoneType::Solid => "solid",
        ZoneType::Column => "column",
        ZoneType::Wall => "wall",
        ZoneType::Opening => "opening",
        ZoneType::AltProduct => "alt_product",
    }
}

pub fn parse_zone_type(name: &str) -> Option<ZoneType> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "exclusion" => ZoneType::Exclusion,
        "solid" => ZoneType::Solid,
        "column" => ZoneType::Column,
        "wall" => ZoneType::Wall,
        "opening" => ZoneType::Opening,
        "alt_product" => ZoneType::AltProduct,
        _ => return None,
    })
}

/// The record's fields after `1001 LPC_UBOT`, checked against the group limits.
fn fields(tag: &UbotTag) -> Result<Vec<RawTag>, String> {
    let s = |v: &str| RawTag { code: 1000, value: v.to_string() };
    let real = |v: f64| RawTag { code: 1040, value: format!("{v}") };
    let mut out = vec![s(VERSION)];
    match tag {
        UbotTag::Instance { instance_id, product_id, layout_id, count_multiplier } => {
            out.extend([s("I"), s(instance_id), s(product_id), s(layout_id), RawTag { code: 1070, value: count_multiplier.to_string() }]);
        }
        UbotTag::Layout { record: r, boundary_handle } => {
            if i32::try_from(r.revision).is_err() {
                return Err(format!("revision {} is larger than {}", r.revision, i32::MAX));
            }
            out.extend([
                s("L"),
                s(&r.layout_id),
                RawTag { code: 1071, value: r.revision.to_string() },
                s(&r.product_id),
                s(&r.rulepack),
                real(r.origin[0]),
                real(r.origin[1]),
                real(r.angle_deg),
                real(r.pitch_x_mm),
                real(r.pitch_y_mm),
                real(r.unit_scale),
                s(&r.geometry_hash),
                s(boundary_handle),
            ]);
        }
        UbotTag::Zone { zone_id, zone_type, clearance_mm, alt_product_id } => {
            out.extend([
                s("Z"),
                s(zone_id),
                s(zone_type_name(*zone_type)),
                real(clearance_mm.unwrap_or(RULEPACK_CLEARANCE)),
                s(alt_product_id.as_deref().unwrap_or("")),
            ]);
        }
    }
    if let Some(n) = out.iter().position(|t| t.code == 1000 && t.value.len() > MAX_STRING_BYTES) {
        return Err(format!("field {} is longer than {MAX_STRING_BYTES} bytes", n + 1));
    }
    Ok(out)
}

/// Positional reader over the fields after `1001 LPC_UBOT`; fields are numbered from 1.
struct Fields<'a> {
    tags: &'a [RawTag],
    at: usize,
}

impl<'a> Fields<'a> {
    fn next(&mut self, code: i32) -> Result<&'a str, String> {
        let n = self.at + 1;
        let t = self.tags.get(self.at).ok_or_else(|| format!("missing field {n}"))?;
        if t.code != code {
            return Err(format!("field {n} has group code {}, expected {code}", t.code));
        }
        self.at += 1;
        Ok(t.value.trim())
    }

    fn text(&mut self) -> Result<String, String> {
        let v = self.next(1000)?;
        if v.len() > MAX_STRING_BYTES {
            return Err(format!("field {} is longer than {MAX_STRING_BYTES} bytes", self.at));
        }
        Ok(v.to_string())
    }

    fn id(&mut self) -> Result<String, String> {
        let v = self.text()?;
        if v.is_empty() { Err(format!("field {} is empty", self.at)) } else { Ok(v) }
    }

    fn real(&mut self) -> Result<f64, String> {
        let v = self.next(1040)?;
        v.parse::<f64>().ok().filter(|x| x.is_finite()).ok_or_else(|| format!("field {} is not a finite number", self.at))
    }

    fn end(&self) -> Result<(), String> {
        if self.at == self.tags.len() { Ok(()) } else { Err(format!("{} fields, expected {}", self.tags.len(), self.at)) }
    }
}

fn parse(tags: &[RawTag]) -> Result<UbotTag, String> {
    let mut f = Fields { tags, at: 0 };
    let version = f.text()?;
    if version != VERSION {
        return Err(format!("unsupported record version '{}'", shown(&version)));
    }
    let tag = match f.text()?.as_str() {
        "I" => {
            let instance_id = f.id()?;
            let product_id = f.id()?;
            let layout_id = f.id()?;
            let count_multiplier = f.next(1070)?.parse::<i16>().map_err(|_| "count multiplier is not an integer".to_string())?;
            if count_multiplier < 1 {
                return Err("count multiplier must be at least 1".into());
            }
            UbotTag::Instance { instance_id, product_id, layout_id, count_multiplier }
        }
        "L" => {
            let layout_id = f.id()?;
            let revision = f
                .next(1071)?
                .parse::<i32>()
                .ok()
                .and_then(|r| u32::try_from(r).ok())
                .ok_or_else(|| "revision is not a 32-bit non-negative integer".to_string())?;
            let product_id = f.id()?;
            let rulepack = f.id()?;
            let origin = [f.real()?, f.real()?];
            let angle_deg = f.real()?;
            let pitch_x_mm = f.real()?;
            let pitch_y_mm = f.real()?;
            let unit_scale = f.real()?;
            let geometry_hash = f.id()?;
            let boundary_handle = f.id()?;
            let record =
                LayoutRecord { layout_id, revision, product_id, rulepack, origin, angle_deg, pitch_x_mm, pitch_y_mm, unit_scale, geometry_hash };
            UbotTag::Layout { record, boundary_handle }
        }
        "Z" => {
            let zone_id = f.id()?;
            let type_name = f.text()?;
            let zone_type = parse_zone_type(&type_name).ok_or_else(|| format!("unknown zone type '{}'", shown(&type_name)))?;
            let clearance = f.real()?;
            let clearance_mm = if clearance == RULEPACK_CLEARANCE {
                None
            } else if clearance >= 0.0 {
                Some(clearance)
            } else {
                return Err("clearance must be -1 (rulepack) or at least 0".into());
            };
            let alt = f.text()?;
            let alt_product_id = (!alt.is_empty()).then_some(alt);
            UbotTag::Zone { zone_id, zone_type, clearance_mm, alt_product_id }
        }
        other => return Err(format!("unknown record kind '{}'", shown(other))),
    };
    f.end()?;
    Ok(tag)
}

//! Millimetres per drawing unit (GEOM-01): from INSUNITS, or the `units` parameter; never guessed.

use crate::{EngineError, Result, Session};

/// An explicit `units` wins over INSUNITS; INSUNITS other than in/mm/cm/m needs one.
pub fn unit_scale(s: &Session, units_param: Option<&str>) -> Result<f64> {
    if let Some(u) = units_param {
        return param_scale(u).ok_or_else(|| {
            let shown: String = u.chars().take(20).collect();
            EngineError::Other(format!("unknown units '{shown}' (use mm, cm, m or in)"))
        });
    }
    let insunits = s.doc()?.header.i64("INSUNITS", 0);
    insunits_scale(insunits)
        .ok_or_else(|| EngineError::Other(format!("the drawing units are not set (INSUNITS = {insunits}); pass units: mm, cm, m or in")))
}

pub fn insunits_scale(insunits: i64) -> Option<f64> {
    match insunits {
        1 => Some(25.4),
        4 => Some(1.0),
        5 => Some(10.0),
        6 => Some(1000.0),
        _ => None,
    }
}

fn param_scale(u: &str) -> Option<f64> {
    match u.trim().to_ascii_lowercase().as_str() {
        "mm" => Some(1.0),
        "cm" => Some(10.0),
        "m" => Some(1000.0),
        "in" => Some(25.4),
        _ => None,
    }
}

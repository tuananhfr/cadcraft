//! FONTS: which fonts the drawing's text styles use, which of them this computer lacks, and what is
//! drawn instead. The replacement table, FONTALT and FONTFALLBACK are user settings (process-wide,
//! like AutoCAD's font mapping file): they decide what is drawn and never change the drawing.
//!
//! `{}` reports. `{"map": {"romans.shx": "Noto Sans"}}` sets entries of the table (an empty value
//! drops one), `{"alt": "Arial"}` sets FONTALT, `{"fallback": "..."}` FONTFALLBACK. The app saves
//! [`profile`] between runs and reads it back with [`restore`].

use cadcraft_fonts::{FontState, LegacyVi, font_info, font_map, set_font_map, ttf};
use serde_json::{Map, Value, json};

use super::*;
use crate::{Result, Session};

/// Table entries [`restore`] reads (a saved profile is untrusted input).
const MAX_ENTRIES: usize = 256;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("fonts", "Fonts...", run_fonts)
            .menu(&["Tools", "Fonts..."])
            .alias(&["fontmap"])
            .params("{map?: {font: replacement | \"\"}, alt?: font, fallback?: \"font, font\"} → styles, missing, map, alt, fallback")
            .enabled(always)
            .noundo(),
    ]
}

fn legacy_label(l: LegacyVi) -> &'static str {
    match l {
        LegacyVi::Tcvn3 { .. } => "TCVN3",
        LegacyVi::Vni => "VNI",
        LegacyVi::Guess { .. } => "TCVN3/VNI",
    }
}

fn state_label(s: FontState) -> &'static str {
    match s {
        FontState::Found => "found",
        FontState::Stroke => "stroke",
        FontState::Missing => "missing",
    }
}

fn map_value() -> Value {
    Value::Object(font_map().into_iter().map(|(k, v)| (k, Value::String(v))).collect::<Map<_, _>>())
}

/// The user's font settings, for saving between runs.
pub fn profile() -> Value {
    json!({"map": map_value(), "alt": ttf::font_alt(), "fallback": ttf::fallback_fonts()})
}

/// Read back a saved [`profile`]. FONTALT and FONTFALLBACK keep their value when the profile has
/// none, so the `CADCRAFT_FONTALT` / `CADCRAFT_FONTFALLBACK` environment still applies.
pub fn restore(saved: &Value) {
    let Some(obj) = saved.as_object() else { return };
    if let Some(m) = obj.get("map").and_then(Value::as_object) {
        set_font_map(m.iter().take(MAX_ENTRIES).filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))));
    }
    match obj.get("alt").and_then(Value::as_str) {
        Some(a) if !a.trim().is_empty() => ttf::set_font_alt(a),
        _ => {}
    }
    match obj.get("fallback").and_then(Value::as_str) {
        Some(f) if !f.trim().is_empty() => ttf::set_fallback_fonts(f),
        _ => {}
    }
}

/// The text styles' fonts and what is drawn for each. `missing` counts the styles in use (by text,
/// dimension styles, ...) whose font this computer lacks: a style nothing uses costs nothing.
pub fn report(s: &Session) -> Value {
    let mut styles = Vec::new();
    let mut missing = 0;
    if let Ok(doc) = s.doc() {
        let in_use = super::props::text_styles_in_use(doc);
        let current = doc.header.str("TEXTSTYLE", "Standard");
        for st in &doc.text_styles {
            let used = st.name.eq_ignore_ascii_case(&current) || in_use.iter().any(|n| n.eq_ignore_ascii_case(&st.name));
            let info = font_info(&st.font, &st.family);
            if used && info.state == FontState::Missing {
                missing += 1;
            }
            styles.push(json!({
                "style": st.name,
                "font": st.font,
                "used": used,
                "state": state_label(info.state),
                "with": info.with,
                "mapped": info.mapped,
                "legacy": info.legacy.map(legacy_label),
            }));
        }
    }
    json!({
        "styles": styles,
        "missing": missing,
        "map": map_value(),
        "alt": ttf::font_alt(),
        "fallback": ttf::fallback_fonts(),
    })
}

fn run_fonts(s: &mut Session, p: &Value) -> Result<Value> {
    let changed = !p.is_null() && p.as_object().is_some_and(|o| ["map", "alt", "fallback"].iter().any(|k| o.contains_key(*k)));
    if !p.is_null() && !p.is_object() {
        return Err(bad("fonts", "expected an object"));
    }
    // Validate everything before touching the process-wide settings: all or nothing.
    let map = match p.get("map") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => {
            let mut next: std::collections::BTreeMap<String, String> = font_map().into_iter().collect();
            for (k, v) in m {
                let v = v.as_str().ok_or_else(|| bad("fonts", format!("the replacement for `{k}` must be text")))?;
                let key = ttf::key(k);
                if key.is_empty() {
                    return Err(bad("fonts", "a font name in `map` is empty"));
                }
                if v.trim().is_empty() {
                    next.remove(&key);
                } else {
                    next.insert(key, v.to_string());
                }
            }
            Some(next)
        }
        Some(_) => return Err(bad("fonts", "`map` must be an object of font → replacement")),
    };
    let text = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone())),
        Some(_) => Err(bad("fonts", format!("`{key}` must be text"))),
    };
    let (alt, fallback) = (text("alt")?, text("fallback")?);
    if let Some(m) = map {
        set_font_map(m);
    }
    if let Some(a) = alt {
        ttf::set_font_alt(&a);
    }
    if let Some(f) = fallback {
        ttf::set_fallback_fonts(&f);
    }
    if changed {
        s.touch();
    }
    Ok(report(s))
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, MutexGuard, PoisonError};

    use super::*;

    /// The replacement table is process-wide: tests that change it run one at a time.
    fn serial() -> MutexGuard<'static, ()> {
        static L: Mutex<()> = Mutex::new(());
        L.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn the_table_round_trips_through_the_profile_and_junk_is_skipped() {
        let _serial = serial();
        set_font_map([("Gone-150.TTF".to_string(), "Arial".to_string())]);
        let saved = profile();
        set_font_map([]);
        restore(&saved);
        assert_eq!(font_map(), vec![("gone-150".to_string(), "Arial".to_string())]);
        restore(&json!({"map": {"x": 3}, "alt": 7}));
        assert!(font_map().is_empty());
        restore(&json!("not an object"));
        set_font_map([]);
    }

    #[test]
    fn the_command_reports_the_styles_and_applies_a_table_all_or_nothing() {
        let _serial = serial();
        set_font_map([]);
        let mut s = Session::new();
        let r = run_fonts(&mut s, &Value::Null).unwrap();
        assert_eq!((r["missing"].as_i64(), r["styles"][0]["state"].as_str()), (Some(0), Some("stroke")));

        assert!(run_fonts(&mut s, &json!({"map": {"a-150.ttf": "Arial", "b-150.ttf": 3}})).is_err());
        assert!(run_fonts(&mut s, &json!({"alt": 3})).is_err());
        assert!(font_map().is_empty(), "a refused call changes nothing");

        let r = run_fonts(&mut s, &json!({"map": {"A-150.TTF": "Noto Sans"}})).unwrap();
        assert_eq!(r["map"]["a-150"], "Noto Sans");
        let r = run_fonts(&mut s, &json!({"map": {"a-150.ttf": ""}})).unwrap();
        assert_eq!(r["map"], json!({}));
    }
}

//! UBLIB: the catalog in use; `load` lays a project or company catalog over the built-in one.

use cadcraft_ubot::Catalog;
use serde_json::{Value, json};

use super::state::UbotState;
use crate::cmd::{CommandSpec, always, bad, str_param};
use crate::{Result, Session};

const CMD: &str = "ublib";
/// Larger catalog files are refused before they are read.
const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new(CMD, "U-BOT Catalog", run)
        .menu(&["Tools", "U-BOT", "Catalog"])
        .params("{action?: list|load|reset (default list), path?: catalog JSON file, text?: catalog JSON} → {catalog, products, rulepacks, source}")
        .enabled(always)
        .noundo()
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    match str_param(p, "action").unwrap_or("list").trim().to_ascii_lowercase().as_str() {
        "list" => {}
        "load" => load(s, p)?,
        "reset" => {
            s.ubot = UbotState::default();
            s.echo("U-BOT catalog reset to the built-in draft catalog.");
        }
        other => {
            let shown: String = other.chars().take(20).collect();
            return Err(bad(CMD, format!("unknown action '{shown}' (use list, load or reset)")));
        }
    }
    let state = &s.ubot;
    let reply = describe(&state.catalog, state.overlay_source.as_deref());
    let lines: Vec<String> = state
        .catalog
        .products
        .iter()
        .map(|p| format!("{}  {}  {}x{} mm  ({:?})", p.id, p.name, p.width_mm, p.length_mm, p.approval_status))
        .collect();
    for line in lines {
        s.echo(line);
    }
    Ok(reply)
}

fn load(s: &mut Session, p: &Value) -> Result<()> {
    let (text, source) = match (str_param(p, "text"), str_param(p, "path")) {
        (Some(text), None) => {
            if text.len() as u64 > MAX_CATALOG_BYTES {
                return Err(bad(CMD, format!("the catalog is larger than {MAX_CATALOG_BYTES} bytes")));
            }
            (text.to_string(), "text".to_string())
        }
        (None, Some(path)) => (read_path(path)?, path.to_string()),
        (Some(_), Some(_)) => return Err(bad(CMD, "pass either `path` or `text`, not both")),
        (None, None) => return Err(bad(CMD, "`load` needs `path` (catalog JSON file) or `text` (catalog JSON)")),
    };
    let overlay = Catalog::from_json(&text).map_err(|e| bad(CMD, e.to_string()))?;
    let catalog = Catalog::builtin().overlay(&overlay);
    s.echo(format!("U-BOT catalog '{}' {} loaded: {} products.", overlay.catalog_id, overlay.version, catalog.products.len()));
    s.ubot = UbotState { catalog, overlay_source: Some(source), ..UbotState::default() };
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_path(path: &str) -> Result<String> {
    let size = std::fs::metadata(path).map_err(|e| bad(CMD, format!("{path}: {e}")))?.len();
    if size > MAX_CATALOG_BYTES {
        return Err(bad(CMD, format!("{path}: the catalog is larger than {MAX_CATALOG_BYTES} bytes")));
    }
    std::fs::read_to_string(path).map_err(|e| bad(CMD, format!("{path}: {e}")))
}

#[cfg(target_arch = "wasm32")]
fn read_path(_path: &str) -> Result<String> {
    Err(bad(CMD, "paths are not available on the web; pass the catalog JSON as `text`"))
}

fn describe(catalog: &Catalog, overlay: Option<&str>) -> Value {
    json!({
        "catalog": {"schema": catalog.schema, "catalogId": catalog.catalog_id, "version": catalog.version},
        "products": catalog.products,
        "rulepacks": catalog.rulepacks,
        "source": overlay.unwrap_or("builtin"),
    })
}

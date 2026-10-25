//! JSON request/response protocol of the `ubot-core` sidecar: one request in, one response out.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::catalog::Catalog;
use crate::check::{CheckRequest, check};
use crate::error::UbotError;
use crate::fill::{FillRequest, fill};
use crate::geometry::RingInput;
use crate::quantity::{CountItem, count_in};
use crate::replace::{ReplaceRequest, replace};

const REQ_SCHEMA: &str = "ubot.req/1";
const RES_SCHEMA: &str = "ubot.res/1";
/// Largest accepted request, in bytes; the sidecar binary reads at most this much plus one byte.
pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountEntry {
    item: CountItem,
    center: [f64; 2],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountRequest {
    unit_scale: f64,
    items: Vec<CountEntry>,
    #[serde(default)]
    boundary: Option<RingInput>,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

enum Failure {
    BadJson(String),
    BadSchema(String),
    UnknownOp(String),
    Core(UbotError),
}

impl From<UbotError> for Failure {
    fn from(e: UbotError) -> Self {
        Self::Core(e)
    }
}

impl Failure {
    fn into_body(self) -> ErrorBody {
        let (code, message) = match self {
            Self::BadJson(m) => ("BAD_JSON", m),
            Self::BadSchema(m) => ("BAD_SCHEMA", m),
            Self::UnknownOp(m) => ("UNKNOWN_OP", m),
            Self::Core(UbotError::BadInput(m)) => ("BAD_INPUT", m),
            Self::Core(UbotError::Limit(m)) => ("LIMIT", m),
            Self::Core(UbotError::UnknownProduct(m)) => ("UNKNOWN_PRODUCT", m),
            Self::Core(UbotError::Geometry(m)) => ("GEOMETRY", m),
        };
        ErrorBody { code, message }
    }
}

/// Handles one request document and returns the single-line response document; never panics.
pub fn handle_json(input: &str) -> String {
    handle_json_status(input).0
}

/// Like [`handle_json`], also reporting whether the response is `ok`.
pub fn handle_json_status(input: &str) -> (String, bool) {
    match run(input) {
        Ok(result) => (json!({ "schema": RES_SCHEMA, "ok": true, "result": result }).to_string(), true),
        Err(f) => (json!({ "schema": RES_SCHEMA, "ok": false, "error": f.into_body() }).to_string(), false),
    }
}

fn run(input: &str) -> Result<Value, Failure> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(UbotError::Limit(format!("request is larger than {MAX_INPUT_BYTES} bytes")).into());
    }
    let mut doc: Value = serde_json::from_str(input).map_err(|e| Failure::BadJson(e.to_string()))?;
    let Some(obj) = doc.as_object_mut() else {
        return Err(Failure::BadSchema("request must be a JSON object".into()));
    };
    match obj.get("schema").and_then(Value::as_str) {
        Some(REQ_SCHEMA) => {}
        _ => return Err(Failure::BadSchema(format!("schema must be '{REQ_SCHEMA}'"))),
    }
    let op = obj.get("op").and_then(Value::as_str).ok_or_else(|| Failure::BadSchema("missing string field 'op'".into()))?.to_string();
    let catalog = effective_catalog(obj)?;
    let request = obj.remove("request");
    dispatch(&op, request, &catalog)
}

fn effective_catalog(obj: &Map<String, Value>) -> Result<Catalog, Failure> {
    let base = Catalog::builtin();
    match obj.get("catalog") {
        None | Some(Value::Null) => Ok(base),
        Some(v) => Ok(base.overlay(&Catalog::from_json(&v.to_string())?)),
    }
}

fn dispatch(op: &str, request: Option<Value>, catalog: &Catalog) -> Result<Value, Failure> {
    match op {
        "version" => {
            Ok(json!({ "core": env!("CARGO_PKG_VERSION"), "schema": RES_SCHEMA, "catalog": format!("{}@{}", catalog.catalog_id, catalog.version) }))
        }
        "catalog" => to_value(catalog),
        "fill" => to_value(&fill(&parse::<FillRequest>(request)?, catalog)?),
        "check" => to_value(&check(&parse::<CheckRequest>(request)?, catalog)?),
        "replace" => to_value(&replace(&parse::<ReplaceRequest>(request)?, catalog)?),
        "count" => {
            let req = parse::<CountRequest>(request)?;
            let items: Vec<(CountItem, [f64; 2])> = req.items.into_iter().map(|e| (e.item, e.center)).collect();
            to_value(&count_in(&items, req.boundary.as_ref().map(|b| (b, req.unit_scale)), catalog)?)
        }
        other => Err(Failure::UnknownOp(format!("unknown op '{other}'"))),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(request: Option<Value>) -> Result<T, Failure> {
    let v = request.ok_or_else(|| UbotError::BadInput("missing 'request'".into()))?;
    serde_json::from_value(v).map_err(|e| UbotError::BadInput(format!("request: {e}")).into())
}

fn to_value<T: Serialize>(v: &T) -> Result<Value, Failure> {
    serde_json::to_value(v).map_err(|e| UbotError::BadInput(format!("result is not serialisable: {e}")).into())
}

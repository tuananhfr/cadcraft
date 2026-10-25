//! JSON protocol of the sidecar: envelope handling and one round trip per interesting op.

use cadcraft_ubot::{MAX_INPUT_BYTES, handle_json};
use serde_json::{Value, json};

fn parse(out: &str) -> Value {
    assert!(!out.contains('\n'), "response must be a single line");
    serde_json::from_str(out).expect("response is JSON")
}

fn call(req: &Value) -> Value {
    parse(&handle_json(&req.to_string()))
}

fn error_code(out: &Value) -> &str {
    assert_eq!(out["ok"], json!(false), "{out}");
    out["error"]["code"].as_str().unwrap_or("")
}

fn square() -> Value {
    json!({"vertices": [[0.0, 0.0, 0.0], [6600.0, 0.0, 0.0], [6600.0, 6600.0, 0.0], [0.0, 6600.0, 0.0]]})
}

fn fill_request() -> Value {
    json!({"unitScale": 1.0, "layoutId": "0123456789abcdef", "boundary": square(), "productId": "ubot-h13-7", "angleDeg": 0.0})
}

#[test]
fn version_op() {
    let out = call(&json!({"schema": "ubot.req/1", "op": "version"}));
    assert_eq!(out["schema"], "ubot.res/1");
    assert_eq!(out["ok"], json!(true));
    assert_eq!(out["result"]["core"], env!("CARGO_PKG_VERSION"));
    assert_eq!(out["result"]["schema"], "ubot.res/1");
    assert_eq!(out["result"]["catalog"], "lpc-builtin@draft-1");
}

#[test]
fn fill_op_round_trips_result() {
    let out = call(&json!({"schema": "ubot.req/1", "op": "fill", "request": fill_request()}));
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["result"]["add"].as_array().map(Vec::len), Some(100), "{out}");
    assert_eq!(out["result"]["layout"]["layoutId"], "0123456789abcdef");
    assert!(out["result"]["add"][0]["instanceId"].as_str().is_some_and(|s| s.starts_with("0123456789abcdef:0:0")));
}

#[test]
fn bad_json_and_unknown_op_and_wrong_schema() {
    assert_eq!(error_code(&parse(&handle_json("{not json"))), "BAD_JSON");
    assert_eq!(error_code(&call(&json!({"schema": "ubot.req/1", "op": "explode"}))), "UNKNOWN_OP");
    assert_eq!(error_code(&call(&json!({"schema": "ubot.req/2", "op": "version"}))), "BAD_SCHEMA");
    assert_eq!(error_code(&call(&json!({"op": "version"}))), "BAD_SCHEMA");
    assert_eq!(error_code(&call(&json!({"schema": "ubot.req/1", "op": "fill"}))), "BAD_INPUT");
    let mut unknown = fill_request();
    unknown["productId"] = json!("nope");
    assert_eq!(error_code(&call(&json!({"schema": "ubot.req/1", "op": "fill", "request": unknown}))), "UNKNOWN_PRODUCT");
    let mut bow_tie = fill_request();
    bow_tie["boundary"] = json!({"vertices": [[0.0, 0.0, 0.0], [6600.0, 6600.0, 0.0], [6600.0, 0.0, 0.0], [0.0, 6600.0, 0.0]]});
    assert_eq!(error_code(&call(&json!({"schema": "ubot.req/1", "op": "fill", "request": bow_tie}))), "GEOMETRY");
}

#[test]
fn catalog_overlay_applies() {
    let overlay = json!({
        "schema": "ubot.catalog/1", "catalogId": "x", "version": "1", "mappings": [], "rulepacks": [],
        "products": [{"id": "acme-1", "sku": "ACME-1", "name": "Acme", "widthMm": 400.0, "lengthMm": 400.0, "version": "1", "approvalStatus": "draft"}]
    });
    let out = call(&json!({"schema": "ubot.req/1", "op": "catalog", "catalog": overlay}));
    assert_eq!(out["ok"], json!(true), "{out}");
    let ids: Vec<&str> = out["result"]["products"].as_array().expect("products").iter().filter_map(|p| p["id"].as_str()).collect();
    assert_eq!(ids, ["ubot-h13-7", "ubot-h10-7", "ubot-h20-7", "ubot-h17-7", "acme-1"]);
    let bad = call(&json!({"schema": "ubot.req/1", "op": "catalog", "catalog": {"schema": "other"}}));
    assert_eq!(error_code(&bad), "BAD_INPUT");
}

#[test]
fn count_op_filters_by_boundary() {
    let item = json!({"productId": "ubot-h13-7", "blockName": "UBOT_H13_7", "multiplier": 3, "source": "tagged"});
    let out = call(&json!({"schema": "ubot.req/1", "op": "count", "request": {
        "unitScale": 1.0,
        "items": [{"item": item, "center": [100.0, 100.0]}, {"item": item, "center": [9000.0, 9000.0]}],
        "boundary": {"vertices": [[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0], [1000.0, 1000.0, 0.0], [0.0, 1000.0, 0.0]]}
    }}));
    assert_eq!(out["ok"], json!(true), "{out}");
    assert_eq!(out["result"]["total"], json!(3));
}

#[test]
fn hostile_inputs_fail_cleanly() {
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    let negative = r#"{"schema":"ubot.req/1","op":"count","request":{"unitScale":1,"items":[{"item":{"blockName":"a","multiplier":-1,"source":"tagged"},"center":[0,0]}]}}"#;
    let cases: [(&str, &str); 8] = [
        ("", "BAD_JSON"),
        ("null", "BAD_SCHEMA"),
        ("[1,2]", "BAD_SCHEMA"),
        (&deep, "BAD_JSON"),
        (r#"{"schema":"ubot.req/1","op":7}"#, "BAD_SCHEMA"),
        (r#"{"schema":"ubot.req/1","op":"fill","request":{"unitScale":1e999}}"#, "BAD_JSON"),
        (r#"{"schema":"ubot.req/1","op":"fill","request":"text"}"#, "BAD_INPUT"),
        (negative, "BAD_INPUT"),
    ];
    for (input, code) in cases {
        assert_eq!(error_code(&parse(&handle_json(input))), code, "input starting {:?}", input.chars().take(40).collect::<String>());
    }
}

#[test]
fn oversize_input_is_limit() {
    let huge = " ".repeat(MAX_INPUT_BYTES + 1);
    assert_eq!(error_code(&parse(&handle_json(&huge))), "LIMIT");
}

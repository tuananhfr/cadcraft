use cadcraft_engine::Session;
use cadcraft_engine::hash::sha256_hex;
use cadcraft_ui_egui::wsprofile::canonical::{CHECKSUM_KEY, ChecksumCheck, canonical_json, checksum_of, verify_checksum, with_checksum};
use cadcraft_ui_egui::wsprofile::startup::{Fallback, fresh_user, parse_set, restore};
use cadcraft_ui_egui::wsprofile::store::{MAX_SET_BYTES, MemoryStore, ProfileSet, ProfileStore};
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};

#[test]
fn canonical_json_sorts_keys_at_every_depth() {
    let value: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":null}}"#).unwrap();
    assert_eq!(canonical_json(&value), r#"{"a":{"c":null,"d":[{"y":2,"z":1}]},"b":1}"#);
}

#[test]
fn checksum_ignores_its_own_field_and_key_order() {
    let first: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":null}}"#).unwrap();
    let second: Value = serde_json::from_str(r#"{"a":{"c":null,"d":[{"y":2,"z":1}]},"b":1}"#).unwrap();
    let checksum = checksum_of(&first);
    assert_eq!(checksum, checksum_of(&second));
    assert_eq!(checksum, format!("sha256:{}", sha256_hex(canonical_json(&first).as_bytes())));

    let mut signed = with_checksum(first);
    assert_eq!(signed.get(CHECKSUM_KEY).and_then(Value::as_str), Some(checksum.as_str()));
    assert_eq!(verify_checksum(&signed), ChecksumCheck::Valid);
    assert_eq!(checksum_of(&signed), checksum);
    assert_eq!(with_checksum(signed.clone()), signed);
    signed[CHECKSUM_KEY] = json!("edited");
    assert_eq!(checksum_of(&signed), checksum);
    assert_eq!(verify_checksum(&signed), ChecksumCheck::Mismatch { stored: "edited".into(), actual: checksum });
}

#[test]
fn checksum_mismatch_is_corrupt() {
    let mut value = with_checksum(serde_json::to_value(ProfileSet::default()).unwrap());
    assert!(parse_set(&value.to_string()).is_ok());
    value["active"] = json!("ARCHITECTURE");
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));

    value["schema_version"] = json!("99.0.0");
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));
    let signed_newer = with_checksum(value);
    assert!(matches!(parse_set(&signed_newer.to_string()), Err(Fallback::NewerMajor(version)) if version == "99.0.0"));
}

#[test]
fn set_without_checksum_still_reads() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/workspace/aged-1y.json")).unwrap();
    let value = json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": fixture});
    assert_eq!(verify_checksum(&value), ChecksumCheck::Missing);
    let set = parse_set(&value.to_string()).unwrap();
    let user = set.user.unwrap();
    assert_eq!(set.active, "SIMPLE");
    assert_eq!(user.profile_id, "user.aged-1y");
    assert_eq!(user.revision, 4);
    assert_eq!(user.schema_version, "1.0.0");
    assert!(parse_set(include_str!("fixtures/workspace/aged-1y.json")).is_ok());
    assert!(parse_set(include_str!("fixtures/workspace/legacy-v1.json")).is_ok());
}

#[test]
fn canonical_json_preserves_array_order_strings_and_numbers() {
    let value: Value = serde_json::from_str(r#"{"z":[true,false,null,1.0,-2,1e30],"a":"quote\" newline\n slash\\ Tiếng Việt"}"#).unwrap();
    let text = canonical_json(&value);
    assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), value);
    assert_eq!(canonical_json(&json!([])), "[]");
    assert_eq!(canonical_json(&json!({})), "{}");
    assert_eq!(canonical_json(&json!([2, 1])), "[2,1]");
    assert_eq!(canonical_json(&json!(1.0)), "1.0");
}

#[test]
fn checksum_includes_nested_checksum_fields() {
    let first = json!({"nested": {"checksum": "first"}});
    let second = json!({"nested": {"checksum": "second"}});
    assert_ne!(checksum_of(&first), checksum_of(&second));
}

#[test]
fn malformed_checksum_is_a_mismatch() {
    for stored in [Value::Null, json!(false), json!(12), json!([]), json!({}), json!("sha256:bad")] {
        let value = json!({"active": "SIMPLE", "checksum": stored});
        assert!(matches!(verify_checksum(&value), ChecksumCheck::Mismatch { .. }));
        assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("checksum")));
    }
}

#[test]
fn commit_writes_valid_checksums_for_current_and_snapshot() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(MemoryStore::default()));
    for _ in 0..2 {
        app.commit_workspace_now().unwrap();
        let store = app.ws.store.as_deref().unwrap();
        let raw = store.read_current().unwrap();
        let value: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(verify_checksum(&value), ChecksumCheck::Valid);
        assert!(value.get("schemaVersion").is_none());
        assert_eq!(value["schema_version"], json!("1.0.0"));
        let set = parse_set(&raw).unwrap();
        assert!(!set.extra.contains_key(CHECKSUM_KEY));
        assert_eq!(store.read_snapshot(set.user.unwrap().revision), Some(raw));
    }
}

#[test]
fn checksum_damage_restores_the_last_valid_snapshot() {
    let mut user = fresh_user("SIMPLE", &[]);
    user.revision = 7;
    let value = with_checksum(json!({"schema_version": "1.0.0", "active": "SIMPLE", "profiles": [], "user": user}));
    let mut damaged = value.clone();
    damaged["active"] = json!("ARCHITECTURE");
    let raw = damaged.to_string();
    let mut store = MemoryStore { current: Some(raw.clone()), ..MemoryStore::default() };
    store.push_snapshot(7, &value.to_string()).unwrap();
    store.push_snapshot(8, &raw).unwrap();

    let restored = restore(&mut store);
    assert!(matches!(restored.fallback, Some(Fallback::RestoredSnapshot { revision: 7, reason }) if reason.contains("checksum")));
    assert_eq!(restored.user.revision, 7);
    assert_eq!(restored.set.active, "SIMPLE");
    assert_eq!(store.read_current(), Some(raw.clone()));
    assert!(store.damaged.values().any(|bytes| bytes == raw.as_bytes()));
}

#[test]
fn oversized_set_is_rejected_before_checksum_verification() {
    let value = json!({"checksum": "invalid", "padding": "x".repeat(MAX_SET_BYTES)});
    assert!(matches!(parse_set(&value.to_string()), Err(Fallback::Corrupt(reason)) if reason.contains("larger than")));
}

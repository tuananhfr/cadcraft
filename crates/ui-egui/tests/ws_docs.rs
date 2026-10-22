//! Workspace contract references and forward-compatibility boundaries.

use std::path::Path;

use cadcraft_ui_egui::wsprofile::migrate::{MigrateError, migrate};
use cadcraft_ui_egui::wsprofile::schema::SCHEMA_VERSION;
use serde_json::{Value, json};

#[test]
fn openapi_references_existing_schema() {
    let docs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/workspace");
    let api = std::fs::read_to_string(docs.join("openapi.yaml")).unwrap().replace("\r\n", "\n");
    let schema_text = std::fs::read_to_string(docs.join("workspace-profile.schema.json")).unwrap();
    let schema: Value = serde_json::from_str(&schema_text).unwrap();

    assert!(api.starts_with("openapi: 3.1.0\n"));
    assert!(api.contains("PROPOSAL awaiting approval"));
    let mut external_refs = 0;
    for line in api.lines() {
        let Some((_, reference)) = line.split_once("$ref:") else { continue };
        let reference = reference.trim().trim_matches(['\'', '"']);
        if let Some(component) = reference.strip_prefix("#/components/") {
            let (category, name) = component.split_once('/').unwrap();
            assert!(api.contains(&format!("\n  {category}:\n")), "missing component category: {reference}");
            assert!(api.contains(&format!("\n    {name}:\n")), "missing component: {reference}");
        } else {
            let (file, fragment) = reference.split_once('#').unwrap_or((reference, ""));
            assert_eq!(file, "./workspace-profile.schema.json");
            assert!(docs.join(file).is_file(), "missing schema file: {reference}");
            assert!(fragment.is_empty() || schema.pointer(fragment).is_some(), "missing schema fragment: {reference}");
            external_refs += 1;
        }
    }
    assert!(external_refs > 0, "the API must reference the existing profile schema");

    let mut pending = vec![&schema];
    while let Some(value) = pending.pop() {
        match value {
            Value::Object(fields) => {
                if let Some(reference) = fields.get("$ref").and_then(Value::as_str) {
                    let pointer = reference.strip_prefix('#').unwrap();
                    assert!(schema.pointer(pointer).is_some(), "missing JSON Schema reference: {reference}");
                }
                pending.extend(fields.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }

    let endpoints: [(&str, &[&str]); 6] = [
        ("", &["get", "post"]),
        ("/{id}", &["get", "patch"]),
        ("/{id}/publish", &["post"]),
        ("/import", &["post"]),
        ("/{id}/export", &["get"]),
        ("/{id}/restore", &["post"]),
    ];
    assert_eq!(api.lines().filter(|line| line.starts_with("  /api/v1/cad-workspaces")).count(), endpoints.len());
    assert_eq!(api.lines().filter(|line| line.trim_start().starts_with("operationId:")).count(), 8);
    for (suffix, methods) in endpoints {
        let marker = format!("  /api/v1/cad-workspaces{suffix}:\n");
        let (_, tail) = api.split_once(&marker).unwrap_or_else(|| panic!("missing section 6 path: {marker}"));
        let section = tail.split("\n  /api/").next().unwrap().split("\ncomponents:").next().unwrap();
        assert!(section.contains(&format!("x-spec-path: /v1/workspaces{suffix}\n")));
        for method in methods {
            assert!(section.contains(&format!("\n    {method}:\n")), "missing {method} {suffix}");
        }
        for status in ["403", "404", "412", "422"] {
            assert!(section.contains(&format!("'{status}':")), "missing {status} for {suffix}");
        }
    }
    assert!(api.contains("name: If-Match"));
    assert!(api.contains("ETag:"));
    assert!(api.contains("effective_policy:"));
    assert!(api.contains("dry_run:"));
}

#[test]
fn future_minor_with_unknown_enum_value_is_invalid() {
    assert_eq!(SCHEMA_VERSION, 3);
    let mut profile: Value = serde_json::from_str(include_str!("fixtures/workspace/future-1.3.0.json")).unwrap();
    profile["schema_version"] = json!("1.3.0");
    assert!(migrate(profile.clone()).is_ok(), "the future minor alone must remain readable");
    profile["owner_type"] = json!("team");
    assert!(matches!(migrate(profile), Err(MigrateError::Invalid(_))));
}

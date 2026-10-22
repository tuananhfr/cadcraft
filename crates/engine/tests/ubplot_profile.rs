use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::codes::PROFILE_LEVEL_SKIPPED;
use cadcraft_engine::sheetplan::ids::canonical_json;
use cadcraft_engine::sheetplan::profile::*;
use cadcraft_engine::sheetplan::queue::{MAX_RETRY_LIMIT, OutputMode, OutputSpec};
use cadcraft_engine::sheetplan::{PlotScale, Severity};
use serde_json::{Value, json};

fn doc(level: ProfileLevel, settings: Value, locks: &[&str]) -> ProfileDoc {
    ProfileDoc {
        schema_version: PRINT_PROFILE_SCHEMA,
        level,
        id: "default".into(),
        revision: 1,
        settings,
        locks: locks.iter().map(|lock| (*lock).to_string()).collect(),
        updated_ms: 1,
    }
}

#[test]
fn user_overrides_company_when_unlocked() {
    let company = doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &[]);
    let user = doc(ProfileLevel::User, json!({"device": "CADCraft PDF"}), &[]);
    let eff = effective(&[company, user]);
    assert_eq!(eff.settings.device, "CADCraft PDF");
    assert_eq!(eff.origin.get("/device"), Some(&ProfileLevel::User));
    assert_eq!(eff.origin.get("/naming"), Some(&ProfileLevel::Platform));
}

#[test]
fn company_lock_wins_over_user() {
    let company = doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &["/device"]);
    let user = doc(ProfileLevel::User, json!({"device": "CADCraft PDF", "paper": "A3"}), &[]);
    let eff = effective(&[company, user]);
    assert_eq!(eff.settings.device, "Plotter A");
    assert_eq!(eff.settings.paper.as_deref(), Some("A3"));
    assert_eq!(eff.origin.get("/device"), Some(&ProfileLevel::Company));
    assert_eq!(eff.locked.get("/device"), Some(&ProfileLevel::Company));
    let error = check_change(&eff, "/device").unwrap_err();
    assert_eq!(error, ProfileLocked { pointer: "/device".into(), level: ProfileLevel::Company });
    assert_eq!(error.to_string(), "Locked by company policy");
    assert!(check_change(&eff, "/paper").is_ok());
}

#[test]
fn precedence_follows_level_order_not_input_order() {
    let user = doc(ProfileLevel::User, json!({"device": "User PDF"}), &[]);
    let platform = doc(ProfileLevel::Platform, json!({"device": "Platform PDF", "paper": "A4"}), &[]);
    let drawing = doc(ProfileLevel::Drawing, json!({"device": "Drawing PDF", "paper": "A3"}), &[]);
    let unsorted = effective(&[user.clone(), platform.clone(), drawing.clone()]);
    let sorted = effective(&[platform, drawing, user]);
    assert_eq!(unsorted, sorted);
    assert_eq!(unsorted.settings.device, "User PDF");
    assert_eq!(unsorted.settings.paper.as_deref(), Some("A3"));
    assert_eq!(unsorted.origin.get("/paper"), Some(&ProfileLevel::Drawing));

    let levels =
        [ProfileLevel::Platform, ProfileLevel::Company, ProfileLevel::Project, ProfileLevel::SheetSet, ProfileLevel::Drawing, ProfileLevel::User];
    for (index, level) in levels.iter().enumerate() {
        let docs: Vec<_> = levels.iter().take(index + 1).rev().map(|level| doc(*level, json!({"device": level.to_string()}), &[])).collect();
        let eff = effective(&docs);
        assert_eq!(eff.settings.device, level.to_string());
        assert_eq!(eff.origin.get("/device"), Some(level));
    }
}

#[test]
fn bad_level_patch_is_skipped_whole() {
    let company = doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &[]);
    let project = doc(ProfileLevel::Project, json!({"device": "Bad Plotter", "pages_per_file": "x"}), &["/paper"]);
    let user = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
    let (eff, issues) = effective_with_issues(&[company, project, user]);
    assert_eq!(eff.settings.device, "Plotter A");
    assert_eq!(eff.settings.pages_per_file, platform_defaults().pages_per_file);
    assert_eq!(eff.settings.paper.as_deref(), Some("A3"));
    assert!(!eff.locked.contains_key("/paper"));
    assert_eq!(eff.origin.get("/device"), Some(&ProfileLevel::Company));
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, PROFILE_LEVEL_SKIPPED);
    assert_eq!(issues[0].severity, Severity::Info);
    assert_eq!(issues[0].sheet_id, None);
    assert!(!issues[0].approved_by_user);
    assert!(issues[0].message.contains("project"));
}

#[test]
fn version_changes_only_with_settings() {
    let original = doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &[]);
    let first = effective(std::slice::from_ref(&original));
    let mut metadata = original.clone();
    metadata.id = "renamed".into();
    metadata.revision = 42;
    metadata.updated_ms = 999;
    metadata.locks.push("/device".into());
    let again = effective(std::slice::from_ref(&metadata));
    assert_eq!(first.version, again.version);
    assert_eq!(first.version.len(), 16);
    assert!(first.version.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    let value = serde_json::to_value(&first.settings).unwrap();
    let expected: String = sha256_hex(canonical_json(&value).as_bytes()).chars().take(16).collect();
    assert_eq!(first.version, expected);
    metadata.settings = json!({"device": "Plotter A", "paper": "A3"});
    assert_ne!(first.version, effective(&[metadata]).version);

    let unknown = doc(ProfileLevel::User, json!({"future_setting": "ignored"}), &[]);
    assert_eq!(effective(&[]).version, effective(&[unknown]).version);
}

#[test]
fn naming_rejects_unknown_tokens() {
    assert!(naming_tokens("{stem}-{date}").is_err());
    assert!(naming_tokens("{stem}-{sheet_id}").is_ok());
    assert!(naming_tokens("{stem}-{sheet_id}-{order}-{part}.pdf").is_ok());
    assert!(naming_tokens("literal-name").is_ok());
    for naming in ["{stem", "stem}", "{}", "{{stem}}", "{STEM}"] {
        assert!(naming_tokens(naming).is_err(), "{naming}");
    }
}

#[test]
fn platform_defaults_match_queue_and_print_contract() {
    let defaults = platform_defaults();
    assert_eq!(defaults.device, "CADCraft PDF");
    assert_eq!(defaults.printer, None);
    assert_eq!(defaults.plot_style_table, "None");
    assert_eq!(defaults.paper, None);
    assert_eq!(defaults.landscape, None);
    assert_eq!(defaults.plot_scale, None);
    assert_eq!(defaults.order, SheetOrder::Pick);
    assert!(defaults.frame_templates.is_empty());
    assert_eq!(defaults.naming, "{stem}");
    assert_eq!(defaults.output_mode, OutputMode::Merged);
    assert_eq!(defaults.pages_per_file, OutputSpec::default().pages_per_file);
    assert_eq!(defaults.pages_per_file, if cfg!(target_arch = "wasm32") { 64 } else { 256 });
    assert!(usize::try_from(defaults.pages_per_file).unwrap() <= cadcraft_doc::MAX_PLOT_PAGES);
    assert!(defaults.pdf_layers);
    assert!(!defaults.text_as_geometry);
    assert!(defaults.lineweights);
    assert_eq!(defaults.retry_limit, 2);
    let (eff, issues) = effective_with_issues(&[]);
    assert_eq!(eff.settings, defaults);
    assert!(issues.is_empty());
    assert!(eff.locked.is_empty());
}

#[test]
fn locks_cover_ancestors_descendants_and_array_replacements() {
    let company = doc(ProfileLevel::Company, json!({"frame_templates": ["approved", "other"]}), &["/frame_templates/0"]);
    for replacement in [json!(["unapproved"]), Value::Null, json!({"0": "unapproved"})] {
        let user = doc(ProfileLevel::User, json!({"frame_templates": replacement, "paper": "A3"}), &[]);
        let (eff, issues) = effective_with_issues(&[company.clone(), user]);
        assert_eq!(eff.settings.frame_templates, vec!["approved", "other"]);
        assert_eq!(eff.settings.paper.as_deref(), Some("A3"));
        assert!(issues.is_empty());
        for pointer in ["", "/frame_templates", "/frame_templates/0", "/frame_templates/0/name"] {
            assert_eq!(check_change(&eff, pointer).unwrap_err().level, ProfileLevel::Company);
        }
        assert!(check_change(&eff, "/frame_templates/01").is_ok());
        assert!(check_change(&eff, "/frame_templates_extra").is_ok());
    }
}

#[test]
fn root_lock_and_lock_owner_are_preserved() {
    let company = doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &[""]);
    let project = doc(ProfileLevel::Project, json!({"device": "Plotter B"}), &[""]);
    let user = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
    let eff = effective(&[user, project, company]);
    assert_eq!(eff.settings.device, "Plotter A");
    assert_eq!(eff.settings.paper, None);
    assert_eq!(eff.locked.get(""), Some(&ProfileLevel::Company));
    assert_eq!(check_change(&eff, "/device").unwrap_err().level, ProfileLevel::Company);
}

#[test]
fn omitted_settings_allow_a_lock_only_level() {
    let company: ProfileDoc = serde_json::from_value(json!({
        "schema_version": 1, "level": "company", "id": "default", "revision": 1, "updated_ms": 0, "locks": ["/device"]
    }))
    .unwrap();
    let user = doc(ProfileLevel::User, json!({"device": "Plotter A"}), &[]);
    let (eff, issues) = effective_with_issues(&[company, user]);
    assert_eq!(eff.settings.device, "CADCraft PDF");
    assert!(issues.is_empty());
    assert_eq!(check_change(&eff, "/device").unwrap_err().level, ProfileLevel::Company);
}

#[test]
fn optional_null_resets_and_arrays_replace() {
    let company = doc(ProfileLevel::Company, json!({"paper": "A3", "landscape": true, "frame_templates": ["a", "b"]}), &[]);
    let user = doc(ProfileLevel::User, json!({"paper": null, "landscape": null, "frame_templates": ["c"]}), &[]);
    let (eff, issues) = effective_with_issues(&[company, user]);
    assert_eq!(eff.settings.paper, None);
    assert_eq!(eff.settings.landscape, None);
    assert_eq!(eff.settings.frame_templates, vec!["c"]);
    assert_eq!(eff.origin.get("/paper"), Some(&ProfileLevel::User));
    assert_eq!(eff.origin.get("/frame_templates"), Some(&ProfileLevel::User));
    assert!(issues.is_empty());
}

#[test]
fn profile_uses_existing_scale_and_output_serialization() {
    let user = doc(ProfileLevel::User, json!({"plot_scale": "1:100", "order": "rows_top_down", "output_mode": "per_sheet", "landscape": true}), &[]);
    let (eff, issues) = effective_with_issues(&[user]);
    assert_eq!(eff.settings.plot_scale, Some(PlotScale::Ratio { paper_mm: 1.0, drawing_units: 100.0 }));
    assert_eq!(eff.settings.output_mode, OutputMode::PerSheet);
    assert_eq!(eff.settings.order, SheetOrder::RowsTopDown);
    assert_eq!(eff.settings.landscape, Some(true));
    assert!(issues.is_empty());
    let value = serde_json::to_value(&eff.settings).unwrap();
    assert_eq!(value["plot_scale"], "1:100");
    assert_eq!(value["output_mode"], "per_sheet");
    assert_eq!(serde_json::to_value(ProfileLevel::SheetSet).unwrap(), "sheet_set");
    assert_eq!(serde_json::to_value(SheetOrder::SheetId).unwrap(), "sheet_id");
    let round_trip: PrintSettings = serde_json::from_value(value).unwrap();
    assert_eq!(round_trip, eff.settings);
}

#[test]
fn invalid_settings_are_skipped_without_partial_application() {
    let max_pages = u32::try_from(cadcraft_doc::MAX_PLOT_PAGES).unwrap();
    for patch in [
        json!({"pages_per_file": 0}),
        json!({"pages_per_file": max_pages + 1}),
        json!({"retry_limit": MAX_RETRY_LIMIT + 1}),
        json!({"plot_scale": "1:0"}),
        json!({"naming": "{unknown}"}),
        json!({"pdf_layers": "yes"}),
        json!({"device": null}),
        json!(["not", "an", "object"]),
    ] {
        let user = doc(ProfileLevel::User, patch, &["/paper"]);
        let (eff, issues) = effective_with_issues(&[user]);
        assert_eq!(eff.settings, platform_defaults());
        assert!(eff.locked.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, PROFILE_LEVEL_SKIPPED);
    }
    let user = doc(ProfileLevel::User, json!({"pages_per_file": max_pages, "retry_limit": MAX_RETRY_LIMIT}), &[]);
    let (eff, issues) = effective_with_issues(&[user]);
    assert_eq!(eff.settings.pages_per_file, max_pages);
    assert_eq!(eff.settings.retry_limit, MAX_RETRY_LIMIT);
    assert!(issues.is_empty());
}

#[test]
fn hostile_profiles_are_bounded_and_skipped_whole() {
    let mut deep = Value::Null;
    for _ in 0..=MAX_PROFILE_DEPTH {
        deep = json!({"nested": deep});
    }
    let oversized = doc(ProfileLevel::User, json!({"device": "x".repeat(MAX_PROFILE_BYTES)}), &[]);
    let deep = doc(ProfileLevel::User, deep, &[]);
    let mut too_many_locks = doc(ProfileLevel::User, json!({"device": "bad"}), &[]);
    too_many_locks.locks = vec!["/device".into(); MAX_PROFILE_LOCKS + 1];
    let mut future = doc(ProfileLevel::User, json!({"device": "bad"}), &[]);
    future.schema_version = PRINT_PROFILE_SCHEMA + 1;
    let invalid_pointer = doc(ProfileLevel::User, json!({"device": "bad"}), &["/bad~escape"]);
    for user in [oversized, deep, too_many_locks, future, invalid_pointer] {
        let (eff, issues) = effective_with_issues(&[user]);
        assert_eq!(eff.settings, platform_defaults());
        assert!(eff.locked.is_empty());
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, PROFILE_LEVEL_SKIPPED);
    }
}

#[test]
fn escaped_pointer_segments_have_exact_boundaries() {
    let company = doc(ProfileLevel::Company, json!({}), &["/name~1child", "/name~0child"]);
    let (eff, issues) = effective_with_issues(&[company]);
    assert!(issues.is_empty());
    assert!(check_change(&eff, "/name~1child/leaf").is_err());
    assert!(check_change(&eff, "/name~0child").is_err());
    assert!(check_change(&eff, "/name/child").is_ok());
    assert!(check_change(&eff, "/name~1children").is_ok());
}

#[test]
fn profile_document_count_is_bounded_with_a_notice() {
    let mut docs = vec![doc(ProfileLevel::User, json!({}), &[]); 256];
    docs.push(doc(ProfileLevel::User, json!({"device": "beyond limit"}), &[]));
    let (eff, issues) = effective_with_issues(&docs);
    assert_eq!(eff.settings, platform_defaults());
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, PROFILE_LEVEL_SKIPPED);
}

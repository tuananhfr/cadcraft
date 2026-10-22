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
        project_id: None,
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

#[test]
fn upgrade_reads_unversioned_doc() {
    let original = doc(ProfileLevel::User, json!({"paper": "A3", "pdf_layers": false}), &["/paper"]);
    let mut value = serde_json::to_value(&original).unwrap();
    assert!(value.get("project_id").is_none());
    value.as_object_mut().unwrap().remove("schema_version");
    let upgraded = migrate(value).unwrap();
    assert_eq!(upgraded.project_id, None);
    assert_eq!(upgraded, original);
    let serialized = serde_json::to_value(&upgraded).unwrap();
    assert!(serialized.get("project_id").is_none());
    assert_eq!(serde_json::from_value::<ProfileDoc>(serialized).unwrap(), upgraded);
}

#[test]
fn store_write_failure_keeps_effective() {
    let original = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
    let mut store = MemoryPrintProfileStore::default();
    let json = serde_json::to_string(&original).unwrap();
    store.write(original.level, &original.id, &json).unwrap();
    store.fail_writes = true;
    let mut session = cadcraft_engine::Session::new();
    session.set_print_profile_store(Box::new(store));
    let before = session.effective_print_profile();
    let changed = doc(ProfileLevel::User, json!({"paper": "A4"}), &[]);
    let error = session.write_print_profile(&changed).unwrap_err();
    assert!(error.contains("print profile store write failed"));
    assert_eq!(session.effective_print_profile(), before);
    assert_eq!(before.settings.paper.as_deref(), Some("A3"));
}

#[test]
fn memory_store_round_trip_keeps_previous_good_and_sorted_ids() {
    let mut store = MemoryPrintProfileStore::default();
    let first = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
    let second = doc(ProfileLevel::User, json!({"paper": "A4"}), &[]);
    let first_json = serde_json::to_string(&first).unwrap();
    store.write(first.level, &first.id, &first_json).unwrap();
    store.write(second.level, &second.id, &serde_json::to_string(&second).unwrap()).unwrap();
    assert_eq!(store.read_last_good(first.level, &first.id).as_deref(), Some(first_json.as_str()));
    assert_eq!(load_doc(&mut store, second.level, &second.id), (Some(second), None));
    for id in ["z-project", "a-project"] {
        let mut project = doc(ProfileLevel::Project, json!({}), &[]);
        project.id = id.into();
        store.write(project.level, id, &serde_json::to_string(&project).unwrap()).unwrap();
    }
    assert_eq!(store.list(ProfileLevel::Project), vec!["a-project", "z-project"]);
    assert_eq!(store.list(ProfileLevel::User), vec!["default"]);
    assert!(store.list(ProfileLevel::Platform).is_empty());
}

#[test]
fn hostile_profile_store_inputs_are_rejected_without_changes() {
    let mut store = MemoryPrintProfileStore::default();
    let original = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
    let json = serde_json::to_string(&original).unwrap();
    store.write(original.level, "default", &json).unwrap();
    let long_id = "a".repeat(65);
    for id in ["", "..", "../a", "a/b", "a\\b", "A", "é", "a.json", "a:b", &long_id] {
        assert!(store.read(ProfileLevel::User, id).is_err(), "{id}");
        assert!(store.write(ProfileLevel::User, id, &json).is_err(), "{id}");
        assert!(store.read_last_good(ProfileLevel::User, id).is_none(), "{id}");
        assert!(store.keep_damaged(ProfileLevel::User, id, b"{").is_err(), "{id}");
    }
    assert!(store.write(ProfileLevel::Platform, "default", &json).is_err());
    let oversized = " ".repeat(MAX_PROFILE_BYTES + 1);
    assert!(store.write(ProfileLevel::User, "default", &oversized).is_err());
    assert!(store.keep_damaged(ProfileLevel::User, "default", oversized.as_bytes()).is_err());
    for settings in [json!({"pages_per_file": 0}), json!({"naming": "{date}"}), json!(7)] {
        let invalid = doc(ProfileLevel::User, settings, &[]);
        assert!(store.write(invalid.level, &invalid.id, &serde_json::to_string(&invalid).unwrap()).is_err());
    }
    assert!(store.write(ProfileLevel::Company, "default", &json).is_err());
    assert!(store.write(ProfileLevel::User, "other", &json).is_err());
    assert_eq!(store.read(ProfileLevel::User, "default").unwrap(), Some(json.as_bytes().to_vec()));
    assert_eq!(load_doc(&mut store, ProfileLevel::User, "default"), (Some(original), None));
}

#[test]
fn migration_rejects_future_schema_and_excessive_depth() {
    let original = doc(ProfileLevel::User, json!({}), &[]);
    let mut future = serde_json::to_value(&original).unwrap();
    future.as_object_mut().unwrap().insert("schema_version".into(), json!(9));
    assert_eq!(migrate(future), Err(MigrateError::Future(9)));
    let mut deep = Value::Null;
    for _ in 0..=MAX_PROFILE_DEPTH {
        deep = json!({"child": deep});
    }
    let mut value = serde_json::to_value(&original).unwrap();
    value.as_object_mut().unwrap().insert("settings".into(), deep);
    assert!(migrate(value).is_err());
}

#[test]
fn drawing_keys_use_title_namespace_and_normalized_native_paths() {
    let hash: String = sha256_hex("Drawing1".as_bytes()).chars().take(16).collect();
    assert_eq!(drawing_key(None, "Drawing1"), format!("name:{hash}"));
    assert_ne!(drawing_key(None, "Drawing1"), drawing_key(None, "Drawing2"));
    #[cfg(not(target_arch = "wasm32"))]
    {
        let expected: String = sha256_hex(b"c:/plans/a.dxf").chars().take(16).collect();
        assert_eq!(drawing_key(Some("C:\\Plans\\A.dxf"), "unused"), expected);
        assert_eq!(drawing_key(Some("c:/plans/./sub/../a.dxf"), "different"), expected);
        assert_ne!(drawing_key(Some("/plans/a.dxf"), "a"), drawing_key(Some("/other/a.dxf"), "a"));
    }
    #[cfg(target_arch = "wasm32")]
    assert_eq!(drawing_key(Some("ignored/path.dxf"), "Drawing1"), format!("name:{hash}"));
    let mut store = MemoryPrintProfileStore::default();
    let mut drawing = doc(ProfileLevel::Drawing, json!({"paper": "A3"}), &[]);
    drawing.id = drawing_key(None, "Drawing1");
    store.write(drawing.level, &drawing.id, &serde_json::to_string(&drawing).unwrap()).unwrap();
    assert_eq!(load_doc(&mut store, drawing.level, &drawing.id), (Some(drawing), None));
}

#[test]
fn session_uses_active_drawing_and_handles_start_tab() {
    let mut session = cadcraft_engine::Session::empty();
    assert_eq!(session.effective_print_profile(), effective(&[]));
    let first = session.new_drawing(false);
    let mut drawing = doc(ProfileLevel::Drawing, json!({"paper": "A3"}), &[]);
    drawing.id = drawing_key(None, &session.docs[first].title);
    session.write_print_profile(&drawing).unwrap();
    assert_eq!(session.effective_print_profile().settings.paper.as_deref(), Some("A3"));
    session.new_drawing(false);
    assert_eq!(session.effective_print_profile().settings.paper, None);
    session.active = first;
    assert_eq!(session.effective_print_profile().settings.paper.as_deref(), Some("A3"));
}

#[test]
fn drawing_project_link_applies_project_layer() {
    let mut session = cadcraft_engine::Session::new();
    let company = doc(ProfileLevel::Company, json!({"device": "Company PDF", "paper": "A4"}), &["/paper"]);
    let mut project = doc(ProfileLevel::Project, json!({"device": "Project PDF", "paper": "A3", "naming": "{sheet_id}"}), &[]);
    project.id = "site-a".into();
    session.write_print_profile(&company).unwrap();
    session.write_print_profile(&project).unwrap();
    session.set_drawing_project(Some("site-a")).unwrap();
    let eff = session.effective_print_profile();
    assert_eq!(eff.settings.device, "Project PDF");
    assert_eq!(eff.origin.get("/device"), Some(&ProfileLevel::Project));
    assert_eq!(eff.settings.paper.as_deref(), Some("A4"));
    assert_eq!(eff.locked.get("/paper"), Some(&ProfileLevel::Company));
    assert_eq!(eff.settings.naming, "{sheet_id}");

    let mut drawing = doc(ProfileLevel::Drawing, json!({"naming": "{order}"}), &[]);
    drawing.id = drawing_key(None, &session.state().unwrap().title);
    drawing.project_id = Some("site-a".into());
    session.write_print_profile(&drawing).unwrap();
    let eff = session.effective_print_profile();
    assert_eq!(eff.settings.naming, "{order}");
    assert_eq!(eff.origin.get("/naming"), Some(&ProfileLevel::Drawing));

    let user = doc(ProfileLevel::User, json!({"device": "User PDF", "paper": "A2"}), &[]);
    session.write_print_profile(&user).unwrap();
    let eff = session.effective_print_profile();
    assert_eq!(eff.settings.device, "User PDF");
    assert_eq!(eff.origin.get("/device"), Some(&ProfileLevel::User));
    assert_eq!(eff.settings.paper.as_deref(), Some("A4"));
    assert!(session.log.is_empty());
}

#[test]
fn unlinked_drawing_skips_project() {
    let mut session = cadcraft_engine::Session::new();
    let company = doc(ProfileLevel::Company, json!({"device": "Company PDF"}), &[]);
    let mut project = doc(ProfileLevel::Project, json!({"device": "Project PDF"}), &[]);
    project.id = "site-a".into();
    session.write_print_profile(&company).unwrap();
    session.write_print_profile(&project).unwrap();
    let expected = effective(&[company]);
    assert_eq!(session.effective_print_profile(), expected);
    let mut drawing = doc(ProfileLevel::Drawing, json!({}), &[]);
    drawing.id = drawing_key(None, &session.state().unwrap().title);
    session.write_print_profile(&drawing).unwrap();
    assert_eq!(session.effective_print_profile(), expected);
    session.set_drawing_project(Some("missing-project")).unwrap();
    assert_eq!(session.effective_print_profile(), expected);
    assert!(session.log.is_empty());
}

#[test]
fn drawing_project_link_can_be_cleared_and_is_scoped_to_active_drawing() {
    let mut session = cadcraft_engine::Session::empty();
    for project_id in [Some("site-a"), None] {
        assert_eq!(session.set_drawing_project(project_id), Err("there is no drawing open".into()));
    }
    let first = session.new_drawing(false);
    let mut project = doc(ProfileLevel::Project, json!({"paper": "A3"}), &[]);
    project.id = "site-a".into();
    session.write_print_profile(&project).unwrap();
    let before = session.state().unwrap().doc.clone();
    let revision = session.state().unwrap().revision;
    session.set_drawing_project(Some("site-a")).unwrap();
    assert_eq!(session.effective_print_profile().settings.paper.as_deref(), Some("A3"));
    assert!(std::sync::Arc::ptr_eq(&before, &session.state().unwrap().doc));
    assert_eq!(session.state().unwrap().revision, revision);
    session.new_drawing(false);
    assert_eq!(session.effective_print_profile(), effective(&[]));
    session.active = first;
    assert_eq!(session.effective_print_profile().settings.paper.as_deref(), Some("A3"));
    session.set_drawing_project(None).unwrap();
    assert_eq!(session.effective_print_profile(), effective(&[]));
    assert!(session.log.is_empty());
}

#[test]
fn drawing_project_link_write_failure_keeps_effective() {
    let mut session = cadcraft_engine::Session::new();
    let mut store = MemoryPrintProfileStore::default();
    let mut drawing = doc(ProfileLevel::Drawing, json!({}), &[]);
    drawing.id = drawing_key(None, &session.state().unwrap().title);
    drawing.project_id = Some("site-a".into());
    let mut project = doc(ProfileLevel::Project, json!({"paper": "A3"}), &[]);
    project.id = "site-a".into();
    for profile in [&drawing, &project] {
        store.write(profile.level, &profile.id, &serde_json::to_string(profile).unwrap()).unwrap();
    }
    store.fail_writes = true;
    session.set_print_profile_store(Box::new(store));
    let before = session.effective_print_profile();
    assert_eq!(before.settings.paper.as_deref(), Some("A3"));
    for project_id in [Some("site-b"), None] {
        assert!(session.set_drawing_project(project_id).unwrap_err().contains("print profile store write failed"));
        assert_eq!(session.effective_print_profile(), before);
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod persistence {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use cadcraft_engine::{Session, doc::Drawing};

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("cadcraft-g6-t2-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn stored(store: &mut dyn PrintProfileStore, profile: &ProfileDoc) {
        store.write(profile.level, &profile.id, &serde_json::to_string(profile).unwrap()).unwrap();
    }

    fn damaged_bytes(temp: &TempDir) -> Vec<Vec<u8>> {
        std::fs::read_dir(temp.0.join("user/damaged")).unwrap().map(|entry| std::fs::read(entry.unwrap().path()).unwrap()).collect()
    }

    #[test]
    fn project_id_rejected_outside_drawing_or_malformed() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let original = doc(ProfileLevel::User, json!({"paper": "A4"}), &[]);
        stored(&mut store, &original);
        let user_bytes = store.read(original.level, &original.id).unwrap();
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(DirPrintProfileStore::new(temp.0.clone())));
        let long_id = "a".repeat(65);
        let invalid_ids = ["Site_A", "", long_id.as_str(), "site/a", "site\\a", "site.a", "site:a", "é"];
        for id in invalid_ids {
            assert!(session.set_drawing_project(Some(id)).is_err(), "{id}");
        }
        assert!(store.list(ProfileLevel::Drawing).is_empty());
        session.set_drawing_project(Some("site-a")).unwrap();
        let id = drawing_key(None, &session.state().unwrap().title);
        let drawing_bytes = store.read(ProfileLevel::Drawing, &id).unwrap();
        let backup = store.read_last_good(ProfileLevel::Drawing, &id);
        let before = session.effective_print_profile();
        for project_id in invalid_ids {
            assert!(session.set_drawing_project(Some(project_id)).is_err(), "{project_id}");
            assert_eq!(store.read(ProfileLevel::Drawing, &id).unwrap(), drawing_bytes);
            assert_eq!(store.read_last_good(ProfileLevel::Drawing, &id), backup);
            let mut invalid = doc(ProfileLevel::Drawing, json!({"device": "Invalid PDF"}), &[]);
            invalid.project_id = Some(project_id.into());
            assert!(migrate(serde_json::to_value(&invalid).unwrap()).is_err());
            assert_eq!(effective_with_issues(&[invalid]).1.len(), 1);
            assert_eq!(session.effective_print_profile(), before);
        }
        for level in [ProfileLevel::Platform, ProfileLevel::Company, ProfileLevel::Project, ProfileLevel::SheetSet, ProfileLevel::User] {
            let mut invalid = original.clone();
            invalid.level = level;
            invalid.project_id = Some("site-a".into());
            assert!(session.write_print_profile(&invalid).is_err());
            assert!(migrate(serde_json::to_value(&invalid).unwrap()).is_err());
            let (eff, issues) = effective_with_issues(&[invalid]);
            assert_eq!(eff, effective(&[]));
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].code, PROFILE_LEVEL_SKIPPED);
        }
        assert_eq!(store.read(original.level, &original.id).unwrap(), user_bytes);
        assert_eq!(store.read(ProfileLevel::Drawing, &id).unwrap(), drawing_bytes);
    }

    #[test]
    fn drawing_project_link_preserves_existing_doc_and_round_trips() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(DirPrintProfileStore::new(temp.0.clone())));
        let id = drawing_key(None, &session.state().unwrap().title);
        session.set_drawing_project(Some("site-a")).unwrap();
        let created = load_doc(&mut store, ProfileLevel::Drawing, &id).0.unwrap();
        assert_eq!(created.schema_version, 1);
        assert_eq!(created.level, ProfileLevel::Drawing);
        assert_eq!(created.id, id);
        assert_eq!(created.project_id.as_deref(), Some("site-a"));
        assert_eq!(created.settings, json!({}));
        assert!(created.locks.is_empty());

        let mut drawing = doc(ProfileLevel::Drawing, json!({"paper": "A3", "pdf_layers": false}), &["/pdf_layers"]);
        drawing.id = id.clone();
        stored(&mut store, &drawing);
        let max_id = "a".repeat(64);
        for project_id in [Some("site-b"), Some("0"), Some(max_id.as_str()), None] {
            session.set_drawing_project(project_id).unwrap();
            let mut expected = drawing.clone();
            expected.project_id = project_id.map(str::to_owned);
            assert_eq!(load_doc(&mut store, ProfileLevel::Drawing, &id), (Some(expected), None));
        }
        let bytes = store.read(ProfileLevel::Drawing, &id).unwrap().unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(value.get("project_id").is_none());
    }

    #[test]
    fn damaged_linked_project_uses_last_good_and_reports_once() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let mut first = doc(ProfileLevel::Project, json!({"paper": "A3"}), &[]);
        first.id = "site-a".into();
        stored(&mut store, &first);
        let mut second = first.clone();
        second.settings = json!({"paper": "A4"});
        stored(&mut store, &second);
        let path = temp.0.join("project/site-a.json");
        std::fs::write(&path, b"{").unwrap();
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(store));
        session.set_drawing_project(Some("site-a")).unwrap();
        let eff = session.effective_print_profile();
        assert_eq!(eff.settings.paper.as_deref(), Some("A3"));
        assert_eq!(eff.origin.get("/paper"), Some(&ProfileLevel::Project));
        assert_eq!(session.log.len(), 1);
        assert!(session.log[0].contains("Print profile project was damaged; restored the last good copy"));
        assert_eq!(session.effective_print_profile(), eff);
        assert_eq!(session.log.len(), 1);
        assert_eq!(std::fs::read(path).unwrap(), b"{");
        let archived: Vec<_> =
            std::fs::read_dir(temp.0.join("project/damaged")).unwrap().map(|entry| std::fs::read(entry.unwrap().path()).unwrap()).collect();
        assert_eq!(archived, vec![b"{".to_vec()]);
    }

    #[test]
    fn profile_survives_restart() {
        let temp = TempDir::new();
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(DirPrintProfileStore::new(temp.0.clone())));
        session.write_print_profile(&doc(ProfileLevel::Company, json!({"device": "Plotter A"}), &["/device"])).unwrap();
        session.write_print_profile(&doc(ProfileLevel::User, json!({"device": "CADCraft PDF", "paper": "A3"}), &[])).unwrap();
        let before = session.effective_print_profile();
        drop(session);
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(DirPrintProfileStore::new(temp.0.clone())));
        assert_eq!(session.effective_print_profile(), before);
        assert_eq!(before.settings.device, "Plotter A");
        assert_eq!(before.settings.paper.as_deref(), Some("A3"));
    }

    #[test]
    fn corrupt_profile_restores_last_good() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let first = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        stored(&mut store, &first);
        stored(&mut store, &doc(ProfileLevel::User, json!({"paper": "A4"}), &[]));
        let path = temp.0.join("user/default.json");
        std::fs::write(&path, b"{").unwrap();
        let (loaded, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(loaded, Some(first));
        assert!(notice.unwrap().contains("restored the last good copy"));
        assert_eq!(std::fs::read(&path).unwrap(), b"{");
        assert_eq!(damaged_bytes(&temp), vec![b"{".to_vec()]);
        let _ = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(damaged_bytes(&temp), vec![b"{".to_vec()]);
    }

    #[test]
    fn missing_current_recovers_last_good_after_interrupted_write() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let good = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        stored(&mut store, &good);
        std::fs::remove_file(temp.0.join("user/default.json")).unwrap();
        let (loaded, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(loaded, Some(good));
        assert!(notice.unwrap().contains("restored the last good copy"));
        assert!(!temp.0.join("user/default.json").exists());
    }

    #[test]
    fn future_schema_kept_as_damaged() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let first = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        stored(&mut store, &first);
        let mut future = serde_json::to_value(doc(ProfileLevel::User, json!({"paper": "A4"}), &[])).unwrap();
        future.as_object_mut().unwrap().insert("schema_version".into(), json!(9));
        let raw = serde_json::to_vec_pretty(&future).unwrap();
        let path = temp.0.join("user/default.json");
        std::fs::write(&path, &raw).unwrap();
        let (loaded, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(loaded, Some(first));
        assert!(notice.unwrap().contains("restored"));
        assert_eq!(std::fs::read(&path).unwrap(), raw);
        assert_eq!(damaged_bytes(&temp), vec![raw]);
    }

    #[test]
    fn drawing_level_never_touches_dwg() {
        let temp = TempDir::new();
        let path = temp.0.join("a.dxf");
        let bytes = b"0\nSECTION\n2\nHEADER\n0\nENDSEC\n0\nEOF\n";
        std::fs::write(&path, bytes).unwrap();
        let root = temp.0.join("settings/print-profiles");
        let mut session = Session::empty();
        let path_text = path.to_string_lossy().into_owned();
        session.open_drawing(Drawing::new_metric(), "a.dxf", Some(path_text.clone()));
        let before = session.state().unwrap().doc.clone();
        let revision = session.state().unwrap().revision;
        session.set_print_profile_store(Box::new(DirPrintProfileStore::new(root.clone())));
        let mut drawing = doc(ProfileLevel::Drawing, json!({"paper": "A3"}), &[]);
        drawing.id = drawing_key(Some(&path_text), "a.dxf");
        session.write_print_profile(&drawing).unwrap();
        assert_eq!(session.effective_print_profile().settings.paper.as_deref(), Some("A3"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(root.join("drawing").join(format!("{}.json", drawing.id)).is_file());
        assert!(std::sync::Arc::ptr_eq(&before, &session.state().unwrap().doc));
        assert_eq!(session.state().unwrap().revision, revision);
    }

    #[test]
    fn damaged_profile_without_backup_is_reported_and_skipped() {
        let temp = TempDir::new();
        std::fs::create_dir(temp.0.join("user")).unwrap();
        std::fs::write(temp.0.join("user/default.json"), b"{").unwrap();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let (loaded, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert!(loaded.is_none());
        assert!(notice.unwrap().contains("no last good copy"));
        assert_eq!(damaged_bytes(&temp), vec![b"{".to_vec()]);
        let mut session = Session::new();
        session.set_print_profile_store(Box::new(store));
        assert_eq!(session.effective_print_profile(), effective(&[]));
        assert_eq!(session.log.len(), 1);
        session.effective_print_profile();
        assert_eq!(session.log.len(), 1);
    }

    #[test]
    fn dir_store_bounds_files_and_rejects_hostile_ids() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let good = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        stored(&mut store, &good);
        for id in ["../a", "a/b", "a\\b", "a:b", "a.json", "..", ""] {
            assert!(store.read(ProfileLevel::User, id).is_err());
            assert!(store.write(ProfileLevel::User, id, "{}").is_err());
            assert!(store.read_last_good(ProfileLevel::User, id).is_none());
            assert!(store.keep_damaged(ProfileLevel::User, id, b"{").is_err());
        }
        let oversized = vec![b' '; MAX_PROFILE_BYTES + 1];
        std::fs::write(temp.0.join("user/default.json"), &oversized).unwrap();
        assert!(store.read(ProfileLevel::User, "default").is_err());
        let (loaded, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(loaded, Some(good));
        assert!(notice.unwrap().contains("restored"));
        assert_eq!(std::fs::read(temp.0.join("user/default.json")).unwrap(), oversized);
    }

    #[test]
    fn stale_temp_file_refuses_write_and_keeps_current() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let good = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        stored(&mut store, &good);
        let path = temp.0.join("user/default.json");
        let before = std::fs::read(&path).unwrap();
        std::fs::write(temp.0.join("user/default.json.part"), b"in-flight writer").unwrap();
        let next = doc(ProfileLevel::User, json!({"paper": "A4"}), &[]);
        assert!(store.write(next.level, &next.id, &serde_json::to_string(&next).unwrap()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert_eq!(load_doc(&mut store, ProfileLevel::User, "default"), (Some(good), None));
    }

    #[test]
    fn drawing_title_keys_have_safe_filenames_and_round_trip_in_list() {
        let temp = TempDir::new();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let mut drawing = doc(ProfileLevel::Drawing, json!({"paper": "A3"}), &[]);
        drawing.id = drawing_key(None, "unsaved drawing");
        stored(&mut store, &drawing);
        let hash = drawing.id.strip_prefix("name:").unwrap();
        assert!(temp.0.join("drawing").join(format!("name--{hash}.json")).is_file());
        assert_eq!(store.list(ProfileLevel::Drawing), vec![drawing.id.clone()]);
        assert!(store.read(ProfileLevel::Drawing, &format!("name--{hash}")).is_err());
        assert_eq!(load_doc(&mut store, drawing.level, &drawing.id), (Some(drawing), None));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_profile_paths_never_modify_external_files() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let outside = TempDir::new();
        let source = outside.0.join("drawing.dxf");
        std::fs::write(&source, b"drawing bytes").unwrap();
        std::fs::create_dir(temp.0.join("user")).unwrap();
        symlink(&source, temp.0.join("user/default.json")).unwrap();
        let mut store = DirPrintProfileStore::new(temp.0.clone());
        let profile = doc(ProfileLevel::User, json!({"paper": "A3"}), &[]);
        assert!(store.read(profile.level, &profile.id).is_err());
        assert!(store.write(profile.level, &profile.id, &serde_json::to_string(&profile).unwrap()).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"drawing bytes");
        let nested = temp.0.join("nested");
        std::fs::create_dir(&nested).unwrap();
        symlink(&outside.0, nested.join("drawing")).unwrap();
        let mut store = DirPrintProfileStore::new(nested);
        let mut drawing = profile;
        drawing.level = ProfileLevel::Drawing;
        assert!(store.write(drawing.level, &drawing.id, &serde_json::to_string(&drawing).unwrap()).is_err());
        assert_eq!(std::fs::read_dir(&outside.0).unwrap().count(), 1);
    }
}

mod commands {
    use super::*;
    use cadcraft_engine::Session;
    use cadcraft_engine::cmd::file::{IoHooks, set_io};

    fn session() -> Session {
        static HOOKS: std::sync::Once = std::sync::Once::new();
        HOOKS.call_once(|| {
            set_io(IoHooks {
                read: |_, _| Err("not needed by the synthetic profile fixture".into()),
                write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
                plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
            });
        });
        let mut session = Session::empty();
        session.open_drawing(cadcraft_doc::Drawing::new_metric(), "x.dxf", None);
        session
    }

    fn scan(session: &mut Session, count: usize) -> Value {
        let windows: Vec<_> = (0..count)
            .map(|index| {
                let x = index as f64 * 500.0;
                json!([[x, 0.0], [x + 420.0, 297.0]])
            })
            .collect();
        let plan = session.execute("ubplot.scan", &json!({"mode": "window", "windows": windows})).unwrap();
        let mut approvals = Vec::new();
        for issue in plan["issues"].as_array().unwrap() {
            if issue["severity"] == "WARN" {
                approvals.push(json!({"sheetKey": "*", "code": issue["code"]}));
            }
        }
        for sheet in plan["sheets"].as_array().unwrap() {
            for issue in sheet["issues"].as_array().unwrap() {
                if issue["severity"] == "WARN" {
                    approvals.push(json!({"sheetKey": format!("{}@{}", sheet["sheet_id"].as_str().unwrap(), sheet["order"]), "code": issue["code"]}));
                }
            }
        }
        session.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap()
    }

    #[test]
    fn effective_profile_applied_to_new_plan() {
        let mut session = session();
        session.write_print_profile(&doc(ProfileLevel::User, json!({"pages_per_file": 64, "naming": "{stem}-{sheet_id}"}), &[])).unwrap();
        let version = session.effective_print_profile().version;
        let mut plan = scan(&mut session, 65);
        assert_eq!(plan["device"], "CADCraft PDF");
        for (index, sheet) in plan["sheets"].as_array_mut().unwrap().iter_mut().enumerate() {
            sheet["sheet_id"] = json!(format!("KT-{:02}", index + 1));
        }
        let result = session.execute("ubplot.publish", &json!({"plan": plan})).unwrap();
        let files = result["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["sheets"].as_array().unwrap().len(), 64);
        assert_eq!(files[1]["sheets"].as_array().unwrap().len(), 1);
        assert_eq!(files[0]["name"], "x-KT-01.pdf");
        assert_eq!(files[1]["name"], "x-KT-65.pdf");
        assert_eq!(result["manifest"]["profile_version"], version);
        let background = session.execute("ubplot.publish", &json!({"plan": plan, "background": true})).unwrap();
        let job = session.take_queued_job().unwrap();
        assert_eq!(background["jobId"], job.job_id);
        assert_eq!(job.output.pages_per_file, 64);
        assert_eq!(job.naming.as_deref(), Some("{stem}-{sheet_id}"));
        assert_eq!(job.profile_version.as_deref(), Some(version.as_str()));
        assert_eq!(job.profile_id.as_deref(), Some("platform>user"));
    }

    #[test]
    fn locked_property_refuses_every_route() {
        let mut session = session();
        session.write_print_profile(&doc(ProfileLevel::Company, json!({"device": "CADCraft PDF"}), &["/device"])).unwrap();
        let effective = session.effective_print_profile();
        let plan = scan(&mut session, 1);
        let assert_locked = |error: String| assert!(error.contains("LOCKED_BY_POLICY: /device (company)"), "{error}");
        assert_locked(
            session.execute("ubplot.profile", &json!({"action": "set", "level": "user", "key": "device", "value": "X"})).unwrap_err().to_string(),
        );
        for background in [false, true] {
            assert_locked(
                session.execute("ubplot.publish", &json!({"plan": plan, "device": "X", "background": background})).unwrap_err().to_string(),
            );
            let mut changed = plan.clone();
            changed["device"] = json!("X");
            assert_locked(session.execute("ubplot.publish", &json!({"plan": changed, "background": background})).unwrap_err().to_string());
        }
        assert_locked(
            session.execute("ubplot.scan", &json!({"mode": "window", "windows": [[[0, 0], [420, 297]]], "device": "X"})).unwrap_err().to_string(),
        );
        let mut changed = plan.clone();
        changed["device"] = json!("X");
        let quoted =
            cadcraft_engine::lisp::value::to_text(&cadcraft_engine::lisp::value::Value::str(&serde_json::to_string(&changed).unwrap()), true);
        let lisp = session.execute("lisp", &json!({"code": format!("(ubplot-publish {quoted} nil)")})).unwrap();
        assert_locked(lisp["error"].as_str().unwrap().to_string());
        let imported = doc(ProfileLevel::User, json!({"device": "X", "naming": "changed"}), &[]);
        assert_locked(
            session
                .execute("ubplot.profile", &json!({"action": "import", "level": "user", "text": serde_json::to_string(&imported).unwrap()}))
                .unwrap_err()
                .to_string(),
        );
        assert_eq!(session.effective_print_profile(), effective);
        assert!(session.execute("ubplot.profile", &json!({"action": "get", "level": "user"})).unwrap()["settings"].as_object().unwrap().is_empty());
        assert!(session.take_queued_job().is_none());
    }

    #[test]
    fn import_rejects_executable_keys() {
        let mut session = session();
        let before = session.effective_print_profile();
        let mut cases = Vec::new();
        for key in ["script", "lisp", "exec", "eval", "code", "onload", "startup", "OnLoad"] {
            let mut settings = json!({"nested": {}});
            settings["nested"][key] = json!("bad");
            cases.push(doc(ProfileLevel::User, settings, &[]));
        }
        for path in ["x.lsp", "x.FAS", "x.vlx", "x.exe", "x.dll"] {
            cases.push(doc(ProfileLevel::User, json!({"nested": [path]}), &[]));
        }
        let mut deep = json!("leaf");
        for _ in 0..33 {
            deep = json!({"nested": deep});
        }
        cases.push(doc(ProfileLevel::User, deep, &[]));
        cases.push(doc(ProfileLevel::User, json!({"naming": "{unknown}"}), &[]));
        cases.push(doc(ProfileLevel::User, json!({"naming": "x".repeat(MAX_PROFILE_BYTES)}), &[]));
        for doc in cases {
            let text = serde_json::to_string(&doc).unwrap();
            assert!(session.execute("ubplot.profile", &json!({"action": "import", "level": "user", "text": text})).is_err());
            assert_eq!(session.effective_print_profile(), before);
        }
    }

    #[test]
    fn explain_reports_origin_level_and_lock() {
        let mut session = session();
        session.write_print_profile(&doc(ProfileLevel::Company, json!({"device": "CADCraft PDF"}), &[])).unwrap();
        session.execute("ubplot.profile", &json!({"action": "lock", "level": "company", "key": "device"})).unwrap();
        session.execute("ubplot.profile", &json!({"action": "set", "key": "/naming", "value": "{stem}-{part}"})).unwrap();
        let explanation = session.execute("ubplot.profile", &json!({"action": "explain"})).unwrap();
        assert_eq!(explanation["origin"]["/device"], "company");
        assert_eq!(explanation["locked"]["/device"], "company");
        assert_eq!(explanation["origin"]["/naming"], "user");
        session.execute("ubplot.profile", &json!({"action": "unset", "key": "naming"})).unwrap();
        assert_eq!(session.effective_print_profile().settings.naming, "{stem}");
        session.execute("ubplot.profile", &json!({"action": "unlock", "key": "device"})).unwrap();
        session.execute("ubplot.profile", &json!({"action": "set", "key": "device", "value": "X"})).unwrap();
        assert_eq!(session.effective_print_profile().settings.device, "X");
    }
}

#[test]
fn mcp_and_lisp_cannot_change_company_policy() {
    let mut session = cadcraft_engine::Session::new();
    session.execute("new", &json!({})).unwrap();
    for params in [
        json!({"action": "lock", "key": "/device"}),
        json!({"action": "set", "level": {"company": null}, "key": "device", "value": "X"}),
        json!({"action": "import", "text": "{\"level\":\"company\",\"settings\":{}}"}),
    ] {
        let error = cadcraft_engine::lisp::trust::with_mcp_input(|| session.execute("ubplot.profile", &params)).unwrap_err().to_string();
        assert!(error.contains("Only the user can change permissions with ubplot.profile"), "{params}: {error}");
    }
    // Reads stay open to MCP; whatever they return, it is not the permission refusal.
    let read = cadcraft_engine::lisp::trust::with_mcp_input(|| session.execute("ubplot.profile", &json!({"action": "effective"})));
    assert!(read.as_ref().err().is_none_or(|error| !error.to_string().contains("Only the user")), "{read:?}");
}

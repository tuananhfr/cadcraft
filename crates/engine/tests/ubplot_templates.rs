#[allow(dead_code)]
mod support;

use cadcraft_engine::Session;
use cadcraft_engine::sheetplan::registry_store::{load_registry, resolve, save_registry};
use cadcraft_engine::sheetplan::{FrameRegistry, REGISTRY_SCHEMA, TemplateStatus, detect_frames};
use serde_json::{Value, json};
use support::frames::{frame_block, place};

fn template(id: &str) -> Value {
    json!({"id": id, "revision": "A", "block_names": ["FRAME"], "geometry": [{"kind": "largest_closed_rect"}]})
}

#[test]
fn registry_v1_migrates_to_v2() {
    for schema in ["1", "1.0"] {
        let text = json!({"schema_version": schema, "templates": [template("office")]}).to_string();
        let (registry, notice) = load_registry(Some(&text));
        assert_eq!(registry.schema_version, REGISTRY_SCHEMA);
        assert!(notice.is_some());
        let item = resolve(&registry, "office@1").unwrap();
        assert_eq!(item.version, 1);
        assert_eq!(item.status, TemplateStatus::Approved);
        assert_eq!(item.revision, "A");
        assert_eq!(item.source, None);
        let text = save_registry(&registry);
        assert_eq!(load_registry(Some(&text)), (registry, None));
    }
}

#[test]
fn superseded_template_still_resolves() {
    let mut old = template("office");
    old["version"] = json!(1);
    old["status"] = json!("superseded");
    old["superseded_by"] = json!("office@2");
    let mut new = template("office");
    new["version"] = json!(2);
    new["status"] = json!("approved");
    new["source"] = json!({"file_name": "synthetic.dxf", "sha256": "a".repeat(64), "imported_ms": 42});
    let (registry, notice) = load_registry(Some(&json!({"schema_version": "2", "templates": [old, new]}).to_string()));
    assert_eq!(notice, None);
    assert_eq!(resolve(&registry, "office@1").unwrap().status, TemplateStatus::Superseded);
    assert_eq!(resolve(&registry, "office@2").unwrap().source.as_ref().unwrap().imported_ms, 42);
    assert_eq!(registry.find("FRAME").unwrap().versioned_id(), "office@2");
    assert!(resolve(&registry, "office@3").is_none());
    assert!(resolve(&registry, "office@-1").is_none());
    let restored = load_registry(Some(&save_registry(&registry))).0;
    assert_eq!(restored, registry);
    assert_eq!(FrameRegistry::builtin().merged_with(restored).templates.len(), 3);
}

#[test]
fn detector_ignores_draft_templates() {
    let mut s = Session::new();
    frame_block(s.doc_mut().unwrap(), "FRAME", 420.0, 297.0);
    let handle = place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let mut draft = template("draft");
    draft["status"] = json!("draft");
    draft["geometry"] = json!([{"kind": "local_rect", "min": [0, 0], "max": [10, 10]}]);
    let mut registry = load_registry(Some(&json!({"schema_version": "2", "templates": [draft]}).to_string())).0;
    let candidates = detect_frames(s.doc().unwrap(), &[handle], &registry);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].template_id, None);
    assert_eq!(candidates[0].outline, None);
    registry = FrameRegistry::builtin().merged_with(registry);
    let candidates = detect_frames(s.doc().unwrap(), &[handle], &registry);
    assert_eq!(candidates[0].template_id.as_deref(), Some("generic@1"));
    assert_eq!(candidates[0].local_mm, [420.0, 297.0]);
    s.ubplot.registry = registry;
    let plan = s.execute("ubplot.scan", &json!({"mode": "block", "handles": [handle.hex()]})).unwrap();
    assert_eq!(plan["sheets"][0]["frame_template_id"], "generic@1");
}

#[test]
fn hostile_registry_never_panics() {
    let templates: Vec<_> = (0..1001).map(|index| template(&format!("office-{index}"))).collect();
    let capped = json!({"schema_version": "2", "templates": templates}).to_string();
    let mut invalid = template("bad-version");
    invalid["version"] = json!(-1);
    let malformed = json!({"schema_version": "2", "templates": [template(""), invalid, template("valid")]}).to_string();
    for (text, expected) in [(capped, 1000), (malformed, 1)] {
        let (registry, notice) = std::panic::catch_unwind(|| load_registry(Some(&text))).expect("registry must not panic");
        assert_eq!(registry.templates.len(), expected);
        assert!(notice.is_some());
        assert_eq!(load_registry(Some(&save_registry(&registry))).0, registry);
    }
    for text in ["{", "null", "[]", r#"{"schema_version":"9","templates":[]}"#] {
        let (registry, notice) = std::panic::catch_unwind(|| load_registry(Some(text))).expect("registry must not panic");
        assert_eq!(registry, FrameRegistry::builtin());
        assert!(notice.is_some());
    }
}

#[test]
fn registry_repairs_superseded_invariant_with_notice() {
    let mut successor = template("has-successor");
    successor["status"] = json!("approved");
    successor["superseded_by"] = json!("new@1");
    let mut missing = template("missing-successor");
    missing["status"] = json!("superseded");
    let (registry, notice) = load_registry(Some(&json!({"schema_version": "2", "templates": [successor, missing]}).to_string()));
    assert!(notice.is_some());
    assert_eq!(registry.templates[0].status, TemplateStatus::Superseded);
    assert_eq!(registry.templates[1].status, TemplateStatus::Draft);
    assert_eq!(load_registry(Some(&save_registry(&registry))), (registry, None));
}

#[test]
fn templates_command_caps_imports_and_lists_versions() {
    let mut s = Session::new();
    let templates: Vec<_> = (0..1001).map(|index| template(&format!("office-{index}"))).collect();
    let imported = json!({"schema_version": "2", "templates": templates});
    let result = s.execute("ubplot.templates", &json!({"action": "import", "text": imported.to_string()})).unwrap();
    assert_eq!(result["templates"].as_array().unwrap().len(), 1000);
    assert_eq!(result["templates"][0]["version"], 1);
    assert_eq!(result["templates"][0]["versionedId"], "office-0@1");
    assert_eq!(result["templates"][0]["status"], "approved");
    assert!(s.log.iter().any(|line| line.contains("1000") && line.contains("dropped")));
    frame_block(s.doc_mut().unwrap(), "FRAME", 420.0, 297.0);
    let handle = place(&mut s, "FRAME", [0.0, 0.0], 1.0, 0.0, false, &[]);
    let candidates = detect_frames(s.doc().unwrap(), &[handle], &s.ubplot.registry);
    assert_eq!(candidates[0].template_id.as_deref(), Some("office-0@1"));
    assert!(candidates[0].outline.is_some());
}

#[test]
fn templates_import_preserves_distinct_versions() {
    let mut s = Session::new();
    let old = json!({"schema_version": "1", "templates": [template("office")]});
    s.execute("ubplot.templates", &json!({"action": "import", "text": old.to_string()})).unwrap();
    let first = resolve(&s.ubplot.registry, "office@1").unwrap().clone();
    let mut second = template("office");
    second["version"] = json!(2);
    second["revision"] = json!("B");
    let newer = json!({"schema_version": "2", "templates": [second]});
    s.execute("ubplot.templates", &json!({"action": "import", "text": newer.to_string()})).unwrap();
    assert_eq!(resolve(&s.ubplot.registry, "office@1"), Some(&first));
    assert_eq!(resolve(&s.ubplot.registry, "office@2").unwrap().revision, "B");
    assert_eq!(s.ubplot.registry.find("FRAME").unwrap().versioned_id(), "office@2");
    assert_eq!(s.ubplot.registry.templates.len(), 3);
}

#[test]
fn registry_store_failure_keeps_session_registry() {
    use cadcraft_engine::sheetplan::registry_store::RegistryStore;

    struct FailingStore;
    impl RegistryStore for FailingStore {
        fn read(&self) -> Result<Option<String>, String> {
            Ok(Some(save_registry(&FrameRegistry::builtin())))
        }
        fn read_last_good(&self) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn write(&mut self, _: &str) -> Result<(), String> {
            Err("synthetic write failure".into())
        }
    }
    let mut s = Session::new();
    s.set_frame_registry_store(Box::new(FailingStore));
    let before = s.ubplot.registry.clone();
    let imported = json!({"schema_version": "2", "templates": [template("office")]});
    let error = s.execute("ubplot.templates", &json!({"action": "import", "text": imported.to_string()})).unwrap_err();
    assert!(error.to_string().contains("synthetic write failure"));
    assert_eq!(s.ubplot.registry, before);
    assert!(s.execute("ubplot.templates", &json!({"action": "reset"})).is_err());
    assert_eq!(s.ubplot.registry, before);
}

#[cfg(not(target_arch = "wasm32"))]
mod desktop {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use cadcraft_engine::sheetplan::registry_store::{DirRegistryStore, RegistryStore, load_stored_registry};

    use super::*;

    struct TempRegistry(PathBuf);
    impl TempRegistry {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            for _ in 0..100 {
                let index = NEXT.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!("cadcraft-registry-{}-{index}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("cannot create registry test directory: {error}"),
                }
            }
            panic!("cannot allocate registry test directory");
        }

        fn store(&self) -> DirRegistryStore {
            DirRegistryStore::new(self.0.clone())
        }
    }
    impl Drop for TempRegistry {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn registry_survives_restart_and_reset_is_persisted() {
        let dir = TempRegistry::new();
        let mut first = Session::new();
        first.set_frame_registry_store(Box::new(dir.store()));
        let imported = json!({"schema_version": "1.0", "templates": [template("office")]});
        first.execute("ubplot.templates", &json!({"action": "import", "text": imported.to_string()})).unwrap();
        let expected = first.ubplot.registry.clone();
        drop(first);
        let mut restarted = Session::new();
        restarted.set_frame_registry_store(Box::new(dir.store()));
        assert_eq!(restarted.ubplot.registry, expected);
        restarted.execute("ubplot.templates", &json!({"action": "reset"})).unwrap();
        let mut third = Session::new();
        third.set_frame_registry_store(Box::new(dir.store()));
        assert_eq!(third.ubplot.registry, FrameRegistry::builtin());
        let backup = dir.store().read_last_good().unwrap().unwrap();
        assert_eq!(load_registry(Some(&backup)).0, expected);
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
    }

    #[test]
    fn corrupt_registry_restores_last_good_without_overwriting_it() {
        let dir = TempRegistry::new();
        let mut store = dir.store();
        let original = save_registry(&FrameRegistry::builtin());
        store.write(&original).unwrap();
        let registry = load_registry(Some(&json!({"schema_version": "2", "templates": [template("office")]}).to_string())).0;
        store.write(&save_registry(&registry)).unwrap();
        for damaged in ["{", r#"{"schema_version":"99","templates":[]}"#] {
            std::fs::write(dir.0.join("frame-templates.json"), damaged).unwrap();
            let (restored, notice) = load_stored_registry(&store);
            assert_eq!(restored, FrameRegistry::builtin());
            assert!(notice.unwrap().contains("restored"));
            assert_eq!(store.read().unwrap().as_deref(), Some(damaged));
            assert_eq!(store.read_last_good().unwrap().as_deref(), Some(original.as_str()));
        }
        store.write(&save_registry(&registry)).unwrap();
        assert_eq!(store.read_last_good().unwrap().as_deref(), Some(original.as_str()));
        assert_eq!(load_stored_registry(&store), (registry, None));
    }

    #[test]
    fn registry_backup_failure_preserves_current_file() {
        let dir = TempRegistry::new();
        let mut store = dir.store();
        let original = save_registry(&FrameRegistry::builtin());
        store.write(&original).unwrap();
        std::fs::create_dir(dir.0.join("frame-templates.json.last-good")).unwrap();
        let registry = load_registry(Some(&json!({"schema_version": "2", "templates": [template("office")]}).to_string())).0;
        assert!(store.write(&save_registry(&registry)).is_err());
        assert_eq!(store.read().unwrap().as_deref(), Some(original.as_str()));
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 2);
    }
}

mod baseline {
    use std::sync::Arc;

    use cadcraft_doc::{Block, Drawing, EntityKind};
    use cadcraft_engine::cmd::file::{IoHooks, base64_encode, set_io};
    use cadcraft_engine::hash::sha256_hex;
    use cadcraft_engine::lisp::read::read_all;
    use cadcraft_engine::sheetplan::template_audit::{AuditReport, ImportPick, audit_template, import_baseline};
    use cadcraft_engine::sheetplan::{GeometryRule, Severity, codes};

    use super::*;
    use support::lpc_fixture::lpc_like_drawing;

    fn hooks() {
        set_io(IoHooks {
            read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
            write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
            plot: None,
        });
    }

    fn fixture_bytes() -> Vec<u8> {
        hooks();
        cadcraft_io::write_dxf(&lpc_like_drawing()).into_bytes()
    }

    fn changed_bytes() -> Vec<u8> {
        let mut drawing = lpc_like_drawing();
        let block = Arc::make_mut(drawing.blocks.get_mut("KHUNG_A1").unwrap());
        let mut outer = (**block.entities.iter().next().unwrap()).clone();
        if let EntityKind::LwPolyline(polyline) = &mut outer.kind {
            for vertex in &mut polyline.vertices {
                if vertex.p.y == 594.0 {
                    vertex.p.y = 600.0;
                }
            }
        }
        assert!(block.entities.replace(outer));
        cadcraft_io::write_dxf(&drawing).into_bytes()
    }

    fn picks() -> Vec<ImportPick> {
        [("KHUNG_A1", "lpc-a1"), ("KHUNG_A3", "lpc-a3")]
            .into_iter()
            .map(|(block, id)| ImportPick {
                block: block.into(),
                template_id: id.into(),
                sheet_id_tags: vec!["SO_BV".into()],
                revision_tags: vec!["REV".into()],
                revision: "R1".into(),
            })
            .collect()
    }

    fn import_params(bytes: &[u8]) -> Value {
        json!({"action": "import", "name": "synthetic.dwt", "data": base64_encode(bytes),
            "sha256": sha256_hex(bytes), "approve": true, "picks": picks()})
    }

    #[test]
    fn audit_reports_blocks_attribs_frames() {
        let bytes = fixture_bytes();
        let report = audit_template(&bytes, "synthetic.dwt").unwrap();
        assert_eq!(report.file_name, "synthetic.dwt");
        assert_eq!(report.sha256, sha256_hex(&bytes));
        assert_eq!(report.insunits, 4);
        assert_eq!(report.dwg_version.as_deref(), Some("AC1015"));
        let block = report.blocks.iter().find(|block| block.name == "KHUNG_A1").unwrap();
        assert_eq!(
            block.attdefs,
            [("SO_BV", "Sheet number"), ("TEN_BV", "Drawing title"), ("TY_LE", "Drawing scale"), ("REV", "Revision"), ("NGAY", "Date")]
                .map(|(tag, prompt)| (tag.to_string(), prompt.to_string()))
                .to_vec()
        );
        assert_eq!(block.closed_outlines, [[841.0, 594.0], [821.0, 574.0]]);
        assert_eq!(block.paper_match, Some(("A1".into(), true)));
        assert_eq!(block.inserts_in_model, 2);
        assert!(block.dynamic);
        assert_eq!(block.anonymous_refs, 1);
        let anonymous = report.blocks.iter().find(|block| block.name == "*U7").unwrap();
        assert_eq!(anonymous.closed_outlines, block.closed_outlines);
        let a3 = report.blocks.iter().find(|block| block.name == "KHUNG_A3").unwrap();
        assert_eq!(a3.closed_outlines.first(), Some(&[420.0, 297.0]));
        assert_eq!(a3.paper_match, Some(("A3".into(), true)));
        assert_eq!(a3.inserts_in_model, 1);
        assert_eq!(block.suggested_rule, Some(GeometryRule::LocalRect { min: [0.0, 0.0], max: [841.0, 594.0] }));
        let mut session = Session::new();
        let result =
            session.execute("ubplot.templates", &json!({"action": "audit", "name": "synthetic.dwt", "data": base64_encode(&bytes)})).unwrap();
        assert_eq!(serde_json::from_value::<AuditReport>(result).unwrap(), report);
    }

    #[test]
    fn import_requires_matching_sha_and_approve() {
        let bytes = fixture_bytes();
        let mut session = Session::new();
        let before = session.ubplot.registry.clone();
        let mut params = import_params(&bytes);
        params["sha256"] = json!("0".repeat(64));
        assert!(session.execute("ubplot.templates", &params).unwrap_err().to_string().contains("sha256"));
        assert_eq!(session.ubplot.registry, before);
        for approve in [Value::Null, json!(false), json!("true")] {
            let mut params = import_params(&bytes);
            params.as_object_mut().unwrap().remove("approve");
            if !approve.is_null() {
                params["approve"] = approve;
            }
            assert!(session.execute("ubplot.templates", &params).unwrap_err().to_string().contains("approve"));
            assert_eq!(session.ubplot.registry, before);
        }
    }

    #[test]
    fn import_creates_versioned_templates() {
        let bytes = fixture_bytes();
        let mut session = Session::new();
        let result = session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        assert_eq!(result["changes"], json!(["lpc-a1@1", "lpc-a3@1"]));
        let item = resolve(&session.ubplot.registry, "lpc-a1@1").unwrap();
        assert_eq!(item.status, TemplateStatus::Approved);
        assert_eq!(item.source.as_ref().unwrap().sha256, sha256_hex(&bytes));
        assert_eq!(item.source.as_ref().unwrap().file_name, "synthetic.dwt");
        assert_eq!(item.attributes.sheet_id, ["SO_BV"]);
        assert_eq!(item.attributes.revision, ["REV"]);
        assert_eq!(item.revision, "R1");
    }

    #[test]
    fn reimport_same_sha_is_noop() {
        let bytes = fixture_bytes();
        let report = audit_template(&bytes, "synthetic.dwt").unwrap();
        let mut registry = FrameRegistry::builtin();
        import_baseline(&mut registry, &report, &picks(), 10).unwrap();
        let before = registry.clone();
        assert_eq!(import_baseline(&mut registry, &report, &picks(), 20).unwrap(), ["lpc-a1 unchanged", "lpc-a3 unchanged"]);
        assert_eq!(registry, before);
        let mut session = Session::new();
        session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        let before = session.ubplot.registry.clone();
        let result = session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        assert_eq!(result["changes"], json!(["lpc-a1 unchanged", "lpc-a3 unchanged"]));
        assert_eq!(session.ubplot.registry, before);
    }

    #[test]
    fn reimport_changed_source_adds_version() {
        let bytes = fixture_bytes();
        let report = audit_template(&bytes, "synthetic.dwt").unwrap();
        let mut registry = FrameRegistry::builtin();
        import_baseline(&mut registry, &report, &picks(), 10).unwrap();
        let first = resolve(&registry, "lpc-a1@1").unwrap().clone();
        let changed = audit_template(&changed_bytes(), "synthetic.dwt").unwrap();
        import_baseline(&mut registry, &changed, &picks(), 20).unwrap();
        let old = resolve(&registry, "lpc-a1@1").unwrap();
        assert_eq!(old.status, TemplateStatus::Superseded);
        assert_eq!(old.superseded_by.as_deref(), Some("lpc-a1@2"));
        assert_eq!(old.geometry, first.geometry);
        assert_eq!(old.source, first.source);
        let new = resolve(&registry, "lpc-a1@2").unwrap();
        assert_eq!(new.status, TemplateStatus::Approved);
        assert_eq!(new.geometry, [GeometryRule::LocalRect { min: [0.0, 0.0], max: [841.0, 600.0] }]);
        assert_eq!(new.source.as_ref().unwrap().sha256, changed.sha256);
        assert_eq!(new.source.as_ref().unwrap().imported_ms, 20);
        assert_eq!(registry.find("KHUNG_A1").unwrap().versioned_id(), "lpc-a1@2");
        assert_eq!(load_registry(Some(&save_registry(&registry))), (registry, None));
    }

    #[test]
    fn scan_uses_imported_template() {
        let bytes = fixture_bytes();
        let mut session = Session::new();
        session.open_drawing(lpc_like_drawing(), "Synthetic", None);
        session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        let handles: Vec<_> = session.doc().unwrap().model.iter().map(|entity| entity.handle.hex()).collect();
        let plan = session.execute("ubplot.scan", &json!({"mode": "block", "handles": handles})).unwrap();
        let sheets = plan["sheets"].as_array().unwrap();
        assert_eq!(sheets.len(), 3);
        for (index, sheet) in sheets.iter().enumerate() {
            assert_eq!(sheet["sheet_id"], format!("KT-{:02}", index + 1));
            assert_eq!(sheet["frame_template_id"], if index < 2 { "lpc-a1@1" } else { "lpc-a3@1" });
            assert_eq!(sheet["source"]["effective_name"], if index < 2 { "KHUNG_A1" } else { "KHUNG_A3" });
            assert_eq!(sheet["source"]["rotation_deg"], if index == 1 { 90.0 } else { 0.0 });
            assert_eq!(sheet["plot_scale"], if index == 2 { "1:2" } else { "1:1" });
        }
    }

    #[test]
    fn hostile_template_file_never_panics() {
        hooks();
        let random: Vec<_> = (0..4096).map(|index| (index * 37 % 256) as u8).collect();
        for bytes in [&[][..], b"not a drawing", random.as_slice()] {
            assert!(std::panic::catch_unwind(|| audit_template(bytes, "hostile.dwt")).unwrap().is_err());
        }
        let mut drawing = Drawing::default();
        for index in 0..10_001 {
            let name = format!("BLOCK_{index}");
            drawing.blocks.insert(name.clone(), Arc::new(Block::new(&name)));
        }
        let bytes = cadcraft_io::write_dxf(&drawing).into_bytes();
        let result = std::panic::catch_unwind(|| audit_template(&bytes, "hostile.dxf")).unwrap();
        match result {
            Err(error) => assert!(error.contains(codes::AUDIT_LIMIT)),
            Ok(report) => assert!(report.issues.iter().any(|issue| issue.code == codes::AUDIT_LIMIT)),
        }
        let mut session = Session::new();
        for params in [
            json!({"action": 1}),
            json!({"action": "audit", "path": 1}),
            json!({"action": "audit", "name": "x.dwt", "data": "!"}),
            json!({"action": "import", "picks": [null], "approve": true}),
        ] {
            assert!(session.execute("ubplot.templates", &params).is_err());
        }
    }

    #[test]
    fn invalid_baseline_picks_leave_registry_unchanged() {
        let bytes = fixture_bytes();
        let report = audit_template(&bytes, "synthetic.dwt").unwrap();
        let mut registry = FrameRegistry::builtin();
        let mut invalid = picks();
        invalid[1].block = "MISSING".into();
        assert!(import_baseline(&mut registry, &report, &invalid, 0).is_err());
        assert_eq!(registry, FrameRegistry::builtin());
        invalid = picks();
        invalid[1].template_id = invalid[0].template_id.clone();
        assert!(import_baseline(&mut registry, &report, &invalid, 0).is_err());
        assert_eq!(registry, FrameRegistry::builtin());
        import_baseline(&mut registry, &report, &picks(), 0).unwrap();
        registry.templates.iter_mut().find(|template| template.id == "lpc-a1").unwrap().version = u32::MAX;
        let before = registry.clone();
        let changed = audit_template(&changed_bytes(), "synthetic.dwt").unwrap();
        assert!(import_baseline(&mut registry, &changed, &picks(), 0).unwrap_err().contains("overflow"));
        assert_eq!(registry, before);
    }

    #[test]
    fn baseline_store_failure_is_atomic_and_unchanged_import_skips_write() {
        use cadcraft_engine::sheetplan::registry_store::RegistryStore;

        struct ReadOnlyStore(FrameRegistry);
        impl RegistryStore for ReadOnlyStore {
            fn read(&self) -> Result<Option<String>, String> {
                Ok(Some(save_registry(&self.0)))
            }
            fn read_last_good(&self) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn write(&mut self, _: &str) -> Result<(), String> {
                Err("synthetic registry write failure".into())
            }
        }
        let bytes = fixture_bytes();
        let report = audit_template(&bytes, "synthetic.dwt").unwrap();
        let mut registry = FrameRegistry::builtin();
        import_baseline(&mut registry, &report, &picks(), 0).unwrap();
        let mut session = Session::new();
        session.set_frame_registry_store(Box::new(ReadOnlyStore(registry.clone())));
        session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        assert_eq!(session.ubplot.registry, registry);
        let error = session.execute("ubplot.templates", &import_params(&changed_bytes())).unwrap_err();
        assert!(error.to_string().contains("synthetic registry write failure"));
        assert_eq!(session.ubplot.registry, registry);
        assert!(resolve(&session.ubplot.registry, "lpc-a1@2").is_none());
    }

    #[test]
    fn exported_lisp_is_readable_and_escapes_strings() {
        let bytes = fixture_bytes();
        let mut session = Session::new();
        session.execute("ubplot.templates", &import_params(&bytes)).unwrap();
        let item = session.ubplot.registry.templates.iter_mut().find(|template| template.id == "lpc-a1").unwrap();
        item.block_names.push("quote\"slash\\line\n()".into());
        let result = session.execute("ubplot.templates", &json!({"action": "exportLisp"})).unwrap();
        let text = result["text"].as_str().unwrap();
        let forms = read_all(text).unwrap();
        assert_eq!(forms.len(), 1);
        assert!(text.starts_with("(setq *ubb-templates* '("));
        assert!(text.contains("lpc-a1@1"));
        assert!(text.contains("SO_BV"));
        assert!(text.contains("841"));
        let evaluated = session.execute("lisp", &json!({"code": text})).unwrap();
        assert!(evaluated.get("error").is_none(), "{evaluated}");
        session.execute("ubplot.templates", &import_params(&changed_bytes())).unwrap();
        let latest = session.execute("ubplot.templates", &json!({"action": "exportLisp"})).unwrap();
        let latest_text = latest["text"].as_str().unwrap();
        assert!(!latest_text.contains("lpc-a1@1"));
        assert!(latest_text.contains("lpc-a1@2"));
        assert_eq!(read_all(latest_text).unwrap().len(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod files {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        use super::*;

        struct SourceDir(PathBuf);
        impl SourceDir {
            fn new() -> Self {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                for _ in 0..100 {
                    let index = NEXT.fetch_add(1, Ordering::Relaxed);
                    let path = std::env::temp_dir().join(format!("cadcraft-baseline-{}-{index}", std::process::id()));
                    match std::fs::create_dir(&path) {
                        Ok(()) => return Self(path),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(error) => panic!("cannot create source directory: {error}"),
                    }
                }
                panic!("cannot allocate source directory");
            }
        }
        impl Drop for SourceDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        #[test]
        fn audit_never_modifies_source() {
            let dir = SourceDir::new();
            let path = dir.0.join("synthetic.dwt");
            let bytes = fixture_bytes();
            std::fs::write(&path, &bytes).unwrap();
            let before = std::fs::metadata(&path).unwrap().modified().unwrap();
            let mut session = Session::new();
            let registry = session.ubplot.registry.clone();
            let state = (session.state().unwrap().revision, session.state().unwrap().is_dirty());
            let report = session.execute("ubplot.templates", &json!({"action": "audit", "path": path})).unwrap();
            assert_eq!(session.ubplot.registry, registry);
            assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
            assert_eq!(sha256_hex(&std::fs::read(&path).unwrap()), sha256_hex(&bytes));
            let params = json!({"action": "import", "path": path, "sha256": report["sha256"], "picks": picks(), "approve": true});
            session.execute("ubplot.templates", &params).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
            assert_eq!(sha256_hex(&std::fs::read(&path).unwrap()), sha256_hex(&bytes));
            assert_eq!((session.state().unwrap().revision, session.state().unwrap().is_dirty()), state);
        }

        #[test]
        fn verify_reports_changed_baseline_and_preserves_registry() {
            let dir = SourceDir::new();
            let path = dir.0.join("synthetic.dwt");
            let bytes = fixture_bytes();
            std::fs::write(&path, &bytes).unwrap();
            let mut session = Session::new();
            session
                .execute(
                    "ubplot.templates",
                    &json!({"action": "import", "path": path, "sha256": sha256_hex(&bytes), "picks": picks(), "approve": true}),
                )
                .unwrap();
            let original = session.ubplot.registry.clone();
            assert_eq!(session.execute("ubplot.templates", &json!({"action": "verify"})).unwrap()["issues"], json!([]));
            let changed = changed_bytes();
            std::fs::write(&path, &changed).unwrap();
            let report = session.execute("ubplot.templates", &json!({"action": "verify"})).unwrap();
            assert!(!report["issues"].as_array().unwrap().is_empty());
            for issue in report["issues"].as_array().unwrap() {
                assert_eq!(issue["code"], codes::BASELINE_CHANGED);
                assert_eq!(serde_json::from_value::<Severity>(issue["severity"].clone()).unwrap(), Severity::Warn);
            }
            assert_eq!(session.ubplot.registry, original);
            assert_eq!(std::fs::read(&path).unwrap(), changed);
        }

        #[test]
        fn export_lisp_writes_output_and_refuses_source_path() {
            let dir = SourceDir::new();
            let path = dir.0.join("synthetic.dwt");
            let output = dir.0.join("templates.lsp");
            let bytes = fixture_bytes();
            std::fs::write(&path, &bytes).unwrap();
            let mut session = Session::new();
            session
                .execute(
                    "ubplot.templates",
                    &json!({"action": "import", "path": path, "sha256": sha256_hex(&bytes), "picks": picks(), "approve": true}),
                )
                .unwrap();
            let result = session.execute("ubplot.templates", &json!({"action": "exportLisp", "path": output})).unwrap();
            assert_eq!(std::fs::read_to_string(output).unwrap(), result["text"].as_str().unwrap());
            let error = session.execute("ubplot.templates", &json!({"action": "exportLisp", "path": path})).unwrap_err();
            assert!(error.to_string().contains(codes::OUTPUT_IS_SOURCE));
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
}

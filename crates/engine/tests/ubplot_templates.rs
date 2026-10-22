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

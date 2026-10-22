//! Where the desktop keeps workspace profiles: the settings directory and the one-time move of the
//! first workspace feature's `workspace.json`.

use std::ffi::OsString;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};

use cadcraft_ui_egui::wsprofile::migrate::from_legacy_v1;
use cadcraft_ui_egui::wsprofile::presets::SAFE_PRESET;
use cadcraft_ui_egui::wsprofile::rfc3339;
use cadcraft_ui_egui::wsprofile::schema::{ProfileStatus, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::startup::parse_set;
use cadcraft_ui_egui::wsprofile::store::{ProfileSet, ProfileStore};
use serde_json::Value;

use crate::profile_store::FileProfileStore;

const LEGACY_FILE: &str = "workspace.json";
const LEGACY_BACKUP: &str = "workspace.v1.bak.json";
/// The old file's own size cap; anything larger was never written by that version.
const LEGACY_MAX_BYTES: u64 = 64 * 1024;

pub struct Opened {
    pub store: FileProfileStore,
    /// Why an old `workspace.json` was not moved; the file is left as it was.
    pub notice: Option<String>,
}

pub fn open_default() -> Option<Opened> {
    config_dir_from(std::env::consts::OS, |key| std::env::var_os(key)).map(|root| open(&root))
}

/// The store under `<root>/workspace/`, after moving a first-generation `<root>/workspace.json` into it.
pub fn open(root: &Path) -> Opened {
    open_with(root, &|from, to| fs::rename(from, to))
}

fn open_with(root: &Path, rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Opened {
    let mut store = FileProfileStore::new(root.join("workspace"));
    let notice = match migrate_legacy(root, &mut store, rename) {
        Ok(notice) => notice,
        Err(error) => Some(format!("Workspace could not be restored; its file was left unchanged. {error}")),
    };
    Opened { store, notice }
}

/// `Ok(Some(notice))`: the workspace was moved, but the old file could not be set aside.
fn migrate_legacy(root: &Path, store: &mut FileProfileStore, rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>) -> Result<Option<String>, String> {
    let legacy = root.join(LEGACY_FILE);
    if !legacy.is_file() || store.has_current() {
        return Ok(None);
    }
    let describe = |error: &dyn std::fmt::Display| format!("{}: {error}", legacy.display());
    let mut bytes = Vec::new();
    File::open(&legacy).and_then(|file| file.take(LEGACY_MAX_BYTES + 1).read_to_end(&mut bytes)).map_err(|error| describe(&error))?;
    if bytes.len() as u64 > LEGACY_MAX_BYTES {
        return Err(describe(&"workspace file exceeds 64 KiB"));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| describe(&format!("invalid workspace JSON: {error}")))?;
    if value.get("version").and_then(Value::as_u64) != Some(1) {
        return Err(describe(&"unsupported workspace version"));
    }
    if !value.get("workspace").is_some_and(Value::is_object) {
        return Err(describe(&"missing workspace object"));
    }
    let mut profile: WorkspaceProfile = serde_json::from_value(from_legacy_v1(&value)).map_err(|error| describe(&error))?;
    let now = rfc3339(cadcraft_ui_egui::now_ms());
    profile.profile_id = format!("user.{:016x}", legacy_profile_hash());
    profile.status = ProfileStatus::Draft;
    profile.created_at = now.clone();
    profile.updated_at = now;
    profile.revision = 1;
    let set = ProfileSet { active: SAFE_PRESET.into(), user: Some(profile), ..ProfileSet::default() };
    let json = serde_json::to_string(&set).map_err(|error| describe(&error))?;
    // Never replace the old file with something startup would refuse to read back.
    parse_set(&json).map_err(|fallback| describe(&format!("{fallback:?}")))?;
    store.push_snapshot(1, &json)?;
    store.write_current(&json)?;
    let backup = backup_path(root);
    // From here the new files are in place and the next start will not migrate again.
    Ok(rename(&legacy, &backup).err().map(|error| format!("Workspace restored; the old file could not be renamed. {}: {error}", legacy.display())))
}

fn legacy_profile_hash() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cadcraft_ui_egui::now_ms().to_bits().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    hasher.finish()
}

/// `workspace.v1.bak.json`, or a time-stamped name when an earlier move already left one.
fn backup_path(root: &Path) -> PathBuf {
    let plain = root.join(LEGACY_BACKUP);
    if !plain.exists() {
        return plain;
    }
    let stamp = (cadcraft_ui_egui::now_ms() / 1000.0) as u64;
    (0..100).map(|n| root.join(format!("workspace.v1.bak.{stamp}-{n}.json"))).find(|path| !path.exists()).unwrap_or(plain)
}

fn config_dir_from(platform: &str, get: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(root) = get("CADCRAFT_CONFIG_DIR") {
        return (!root.is_empty()).then(|| PathBuf::from(root));
    }
    let nonempty = |key| get(key).filter(|value| !value.is_empty()).map(PathBuf::from);
    match platform {
        "macos" => nonempty("HOME").map(|home| home.join("Library/Application Support/UbotCAD")),
        "windows" => nonempty("APPDATA").map(|root| root.join("UbotCAD")),
        _ => nonempty("XDG_CONFIG_HOME").map(|root| root.join("ubotcad")).or_else(|| nonempty("HOME").map(|home| home.join(".config/ubotcad"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_store::FileProfileStore;
    use cadcraft_ui_egui::CadApp;
    use cadcraft_ui_egui::wsprofile::store::ProfileStore;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cadcraft-workspace-test-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn legacy_file(root: &Path) -> Vec<u8> {
        let bytes = serde_json::to_vec_pretty(&json!({
            "version": 1,
            "workspace": {"showToolbar": false, "toolsetTab": "Modeling", "historyLines": 7, "collapsedGroups": ["Draw"]}
        }))
        .unwrap();
        fs::write(root.join("workspace.json"), &bytes).unwrap();
        bytes
    }

    fn app() -> CadApp {
        CadApp::new(cadcraft_engine::Session::default(), Default::default())
    }

    #[test]
    fn profile_override_isolated_or_explicitly_disabled() {
        let get = |key: &str| match key {
            "CADCRAFT_CONFIG_DIR" => Some(OsString::from("/private/test-state")),
            "HOME" | "APPDATA" | "XDG_CONFIG_HOME" => Some(OsString::from("/unrelated-user-profile")),
            _ => None,
        };
        for platform in ["macos", "windows", "linux", "freebsd"] {
            assert_eq!(config_dir_from(platform, get), Some(PathBuf::from("/private/test-state")));
            assert_eq!(config_dir_from(platform, |key| if key == "CADCRAFT_CONFIG_DIR" { Some(OsString::new()) } else { get(key) }), None);
        }
        assert_eq!(
            config_dir_from("linux", |key| (key == "XDG_CONFIG_HOME").then(|| OsString::from("/config"))),
            Some(PathBuf::from("/config/ubotcad"))
        );
        for platform in ["linux", "freebsd"] {
            assert_eq!(
                config_dir_from(platform, |key| (key == "HOME").then(|| OsString::from("/home/u"))),
                Some(PathBuf::from("/home/u/.config/ubotcad"))
            );
        }
        assert_eq!(
            config_dir_from("macos", |key| (key == "HOME").then(|| OsString::from("/Users/u"))),
            Some(PathBuf::from("/Users/u/Library/Application Support/UbotCAD"))
        );
        assert_eq!(
            config_dir_from("windows", |key| (key == "APPDATA").then(|| OsString::from(r"C:\Users\u\AppData\Roaming"))),
            Some(PathBuf::from(r"C:\Users\u\AppData\Roaming").join("UbotCAD"))
        );
        assert_eq!(config_dir_from("macos", |_| None), None);
    }

    #[test]
    fn file_store_atomic_and_migrates_legacy() {
        let root = temp_dir();
        let legacy = legacy_file(&root);
        let opened = open(&root);
        assert_eq!(opened.notice, None);
        let mut cad = app();
        cad.attach_profile_store(Box::new(opened.store));
        assert_eq!(cad.ws.user.revision, 1);
        assert!(cad.ws.user.profile_id.starts_with("user.") && cad.ws.user.profile_id != "user.legacy-v1", "{}", cad.ws.user.profile_id);
        assert!(!cad.ws.user.created_at.starts_with("1970"), "{}", cad.ws.user.created_at);
        let standard = cad.ws.effective.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap();
        assert!(!standard.visible, "showToolbar:false must survive the move");
        assert_eq!(cad.ws.effective.layout.toolsets.tab, "Modeling");
        assert_eq!(cad.ws.effective.layout.command_line.lines, 7);
        assert!(cad.ws.fallback.is_none(), "{:?}", cad.ws.fallback);

        let workspace = root.join("workspace");
        let first: Value = serde_json::from_str(&fs::read_to_string(workspace.join("current.json")).unwrap()).unwrap();
        assert_eq!(first["user"]["revision"], 1);
        assert_eq!(fs::read(root.join("workspace.v1.bak.json")).unwrap(), legacy);
        assert!(!root.join("workspace.json").exists());
        assert!(workspace.join("snapshots/rev-1.json").exists());

        // Next start: nothing left to migrate, and the stored revision keeps counting up.
        let again = open(&root);
        assert_eq!(again.notice, None);
        let mut cad = app();
        cad.attach_profile_store(Box::new(again.store));
        assert_eq!(cad.ws.user.revision, 1);
        cad.commit_workspace_now().unwrap();
        assert_eq!(cad.ws.user.revision, 2);
        assert!(!cad.ws.effective.layout.toolbars.iter().find(|toolbar| toolbar.id == "standard").unwrap().visible);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_file_renamed_after_migration() {
        let root = temp_dir();
        legacy_file(&root);
        let opened = open(&root);
        assert!(opened.store.has_current());
        assert!(root.join("workspace.v1.bak.json").is_file());
        assert!(!root.join("workspace.json").exists());
        // A backup from an earlier move is not overwritten.
        fs::remove_file(root.join("workspace/current.json")).unwrap();
        let earlier = fs::read(root.join("workspace.v1.bak.json")).unwrap();
        fs::write(root.join("workspace.json"), &earlier).unwrap();
        let opened = open(&root);
        assert_eq!(opened.notice, None);
        assert!(!root.join("workspace.json").exists());
        let backups =
            fs::read_dir(&root).unwrap().flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with("workspace.v1.bak")).count();
        assert_eq!(backups, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_failed_rename_is_reported_apart_from_a_failed_restore() {
        let root = temp_dir();
        let legacy = legacy_file(&root);
        let opened = open_with(&root, &|_, _| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read-only")));
        let notice = opened.notice.unwrap();
        assert!(notice.starts_with("Workspace restored; the old file could not be renamed"), "{notice}");
        assert!(!notice.contains("could not be restored"), "{notice}");
        assert!(opened.store.has_current());
        assert!(root.join("workspace/snapshots/rev-1.json").exists());
        assert_eq!(fs::read(root.join("workspace.json")).unwrap(), legacy);
        // The next start uses the new set and does not move the file again.
        let again = open(&root);
        assert_eq!(again.notice, None);
        assert!(root.join("workspace.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unusable_legacy_files_are_reported_and_left_unchanged() {
        let root = temp_dir();
        let path = root.join("workspace.json");
        for bytes in [b"{invalid".to_vec(), vec![b' '; 64 * 1024 + 1], br#"{"version":2,"workspace":{}}"#.to_vec(), br#"{"version":1}"#.to_vec()] {
            fs::write(&path, &bytes).unwrap();
            let opened = open(&root);
            assert!(opened.notice.as_deref().is_some_and(|notice| notice.contains("left unchanged")), "{:?}", opened.notice);
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(!opened.store.has_current());
            assert!(!root.join("workspace.v1.bak.json").exists());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_existing_current_set_wins_over_a_legacy_file() {
        let root = temp_dir();
        let mut store = FileProfileStore::new(root.join("workspace"));
        store.write_current(r#"{"active":"SIMPLE"}"#).unwrap();
        let legacy = legacy_file(&root);
        let opened = open(&root);
        assert_eq!(opened.notice, None);
        assert_eq!(fs::read(root.join("workspace.json")).unwrap(), legacy);
        assert_eq!(opened.store.read_current().as_deref(), Some(r#"{"active":"SIMPLE"}"#));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn changes_survive_a_restart_through_the_files() {
        let root = temp_dir();
        let mut cad = app();
        cad.attach_profile_store(Box::new(open(&root).store));
        cad.run("ui.workspace.set", json!({"pointer": "/ui_scale", "value": 1.25})).unwrap();
        cad.commit_workspace_now().unwrap();
        drop(cad);
        let mut cad = app();
        cad.attach_profile_store(Box::new(open(&root).store));
        assert_eq!(cad.ws.user.ui_scale, 1.25);
        assert!(cad.ws.fallback.is_none(), "{:?}", cad.ws.fallback);
        fs::remove_dir_all(root).unwrap();
    }
}

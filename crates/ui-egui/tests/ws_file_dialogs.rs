use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::WORKSPACE_FILE;
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};

struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cadcraft-ws-file-dialogs-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join("Workspace.json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn drawing_dialogs(path: &Path, calls: &Rc<Cell<u32>>) -> Services {
    let open_path = path.to_string_lossy().to_string();
    let save_path = open_path.clone();
    let open_calls = Rc::clone(calls);
    let save_calls = Rc::clone(calls);
    Services {
        pick_open: Some(Box::new(move || {
            open_calls.set(open_calls.get() + 1);
            Some(open_path.clone())
        })),
        pick_save: Some(Box::new(move |_| {
            save_calls.set(save_calls.get() + 1);
            Some(save_path.clone())
        })),
        ..Services::default()
    }
}

#[test]
fn export_uses_the_workspace_save_dialog() {
    let dir = TempDir::new("export");
    let path = dir.file();
    let calls = Rc::new(Cell::new(0));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&picked);
    let save_path = path.to_string_lossy().to_string();
    let mut services = drawing_dialogs(&path, &calls);
    services.pick_save_kind = Some(Box::new(move |name, kind| {
        recorded.borrow_mut().push((name.to_string(), *kind));
        Some(save_path.clone())
    }));
    let mut app = CadApp::new(Session::new(), services);

    let exported = app.run("ui.workspace.export", Value::Null).unwrap();

    assert_eq!(*picked.borrow(), vec![("Workspace.json".to_string(), WORKSPACE_FILE)]);
    assert_eq!(calls.get(), 0);
    assert_eq!(exported["path"].as_str(), path.to_str());
    assert!(path.exists());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), exported["text"].as_str().unwrap());
}

#[test]
fn import_uses_the_workspace_open_dialog() {
    let dir = TempDir::new("import");
    let path = dir.file();
    let calls = Rc::new(Cell::new(0));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&picked);
    let open_path = path.to_string_lossy().to_string();
    let mut services = drawing_dialogs(&path, &calls);
    services.pick_open_kind = Some(Box::new(move |kind| {
        recorded.borrow_mut().push(*kind);
        Some(open_path.clone())
    }));
    let mut app = CadApp::new(Session::new(), services);
    let exported = app.run("ui.workspace.export", json!({})).unwrap();
    std::fs::write(&path, exported["text"].as_str().unwrap()).unwrap();

    let before = app.ws.user.clone();
    let imported = app.run("ui.workspace.import", Value::Null).unwrap();

    assert_eq!(imported["dry_run"], true);
    assert_eq!(app.ui.dialog.as_deref(), Some(cadcraft_ui_egui::wsprofile::import_preview::DIALOG));
    assert_eq!(app.ws.user, before);
    assert_eq!(*picked.borrow(), vec![WORKSPACE_FILE]);
    assert_eq!(calls.get(), 0);
}

#[test]
fn cancelling_the_workspace_dialog_does_not_fall_back() {
    let dir = TempDir::new("cancel");
    let path = dir.file();
    let calls = Rc::new(Cell::new(0));
    let saves = Rc::new(Cell::new(0));
    let opens = Rc::new(Cell::new(0));
    let save_calls = Rc::clone(&saves);
    let open_calls = Rc::clone(&opens);
    let mut services = drawing_dialogs(&path, &calls);
    services.pick_save_kind = Some(Box::new(move |_, _| {
        save_calls.set(save_calls.get() + 1);
        None
    }));
    services.pick_open_kind = Some(Box::new(move |_| {
        open_calls.set(open_calls.get() + 1);
        None
    }));
    let mut app = CadApp::new(Session::new(), services);

    let exported = app.run("ui.workspace.export", Value::Null).unwrap();
    let imported = app.run("ui.workspace.import", Value::Null).unwrap();

    assert_eq!(exported["path"], Value::Null);
    assert_eq!(imported, Value::Null);
    assert_eq!(saves.get(), 1);
    assert_eq!(opens.get(), 1);
    assert_eq!(calls.get(), 0);
    assert!(!path.exists());
}

#[test]
fn hosts_without_workspace_dialogs_keep_the_drawing_dialogs() {
    let dir = TempDir::new("fallback");
    let path = dir.file();
    let open_path = path.to_string_lossy().to_string();
    let save_path = open_path.clone();
    let saves = Rc::new(RefCell::new(Vec::new()));
    let opens = Rc::new(Cell::new(0));
    let save_names = Rc::clone(&saves);
    let open_calls = Rc::clone(&opens);
    let services = Services {
        pick_open: Some(Box::new(move || {
            open_calls.set(open_calls.get() + 1);
            Some(open_path.clone())
        })),
        pick_save: Some(Box::new(move |name| {
            save_names.borrow_mut().push(name.to_string());
            Some(save_path.clone())
        })),
        ..Services::default()
    };
    let mut app = CadApp::new(Session::new(), services);

    let exported = app.run("ui.workspace.export", Value::Null).unwrap();
    let imported = app.run("ui.workspace.import", Value::Null).unwrap();

    assert_eq!(*saves.borrow(), vec!["Workspace.json".to_string()]);
    assert_eq!(opens.get(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), exported["text"].as_str().unwrap());
    assert_ne!(imported, Value::Null);
}

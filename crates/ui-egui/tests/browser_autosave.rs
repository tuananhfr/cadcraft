//! Browser drawing persistence remains independent of workspace commits and recovery.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::Session;
use cadcraft_engine::autosave::{self, Recoverable};
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_ui_egui::autosave::{AutosaveSink, MAX_WEB_AUTOSAVE_BYTES};
use cadcraft_ui_egui::wsprofile::save_state::DrawingSave;
use cadcraft_ui_egui::wsprofile::startup::{parse_set, set_text};
use cadcraft_ui_egui::wsprofile::store::MemoryStore;
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::json;

struct FakeSink(Rc<RefCell<Storage>>);

struct Storage {
    writes: Vec<(String, String, Vec<u8>)>,
    deletes: Vec<String>,
    beats: Vec<(String, f64)>,
    files: BTreeMap<String, Vec<u8>>,
    durable: BTreeMap<String, f64>,
    fetched: Option<(String, Result<Vec<u8>, String>)>,
    auto_complete: bool,
    limit: usize,
    fail_write: bool,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            writes: Vec::new(),
            deletes: Vec::new(),
            beats: Vec::new(),
            files: BTreeMap::new(),
            durable: BTreeMap::new(),
            fetched: None,
            auto_complete: true,
            limit: MAX_WEB_AUTOSAVE_BYTES,
            fail_write: false,
        }
    }
}

impl AutosaveSink for FakeSink {
    fn write(&mut self, name: &str, title: &str, bytes: Vec<u8>) -> Result<(), String> {
        let mut storage = self.0.borrow_mut();
        if storage.fail_write {
            return Err("Browser storage is full".into());
        }
        storage.writes.push((name.into(), title.into(), bytes.clone()));
        storage.files.insert(name.into(), bytes);
        if storage.auto_complete {
            let at = storage.durable.get(name).copied().unwrap_or(1_800_000_000_000.0) + 1.0;
            storage.durable.insert(name.into(), at);
        }
        Ok(())
    }
    fn delete(&mut self, name: &str) {
        let mut storage = self.0.borrow_mut();
        storage.deletes.push(name.into());
        storage.files.remove(name);
        storage.durable.remove(name);
    }
    fn beat(&mut self, instance: &str, now_ms: f64) {
        self.0.borrow_mut().beats.push((instance.into(), now_ms));
    }
    fn fetch(&mut self, name: &str) {
        let mut storage = self.0.borrow_mut();
        let result = storage.files.get(name).cloned().ok_or_else(|| "missing autosave".into());
        storage.fetched = Some((name.into(), result));
    }
    fn take_fetched(&mut self) -> Option<(String, Result<Vec<u8>, String>)> {
        self.0.borrow_mut().fetched.take()
    }
    fn last_written(&self, name: &str) -> Option<f64> {
        self.0.borrow().durable.get(name).copied()
    }
    fn max_bytes(&self) -> usize {
        self.0.borrow().limit
    }
}

fn setup() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    autosave::set_savetime(1);
    set_io(IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|e| e.to_string()),
        write: |doc, name| cadcraft_io::write(doc, name).map_err(|e| e.to_string()),
        plot: None,
    });
    guard
}

fn browser(items: Vec<Recoverable>, storage: Rc<RefCell<Storage>>) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.enable_browser_autosave(Box::new(FakeSink(storage)), "123-ab".into(), items);
    app
}

fn line(app: &mut CadApp) {
    app.run("line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
}

fn frame(app: &mut CadApp, ctx: &egui::Context, time: f64) {
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| app.autosave_tick(ui.ctx()));
    out.textures_delta.clear();
}

fn recovery(storage: &Rc<RefCell<Storage>>, uid: u64) -> Recoverable {
    let mut source = CadApp::new(Session::new(), Services::default());
    line(&mut source);
    let bytes = cadcraft_io::write(source.session.doc().unwrap(), "old.dxf").unwrap();
    let item = Recoverable {
        path: autosave::file_name("Old.dxf", "42-a", uid),
        drawing: "Old.dxf".into(),
        modified: 1_800_000_000,
        bytes: bytes.len() as u64,
    };
    storage.borrow_mut().files.insert(item.path.clone(), bytes);
    item
}

fn workspace() -> MemoryStore {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.attach_profile_store(Box::<MemoryStore>::default());
    app.run("ui.toggle.toolbar", json!({"on": false})).unwrap();
    app.commit_workspace_now().unwrap();
    let raw = app.ws.store.as_ref().unwrap().read_current().unwrap();
    let mut set = parse_set(&raw).unwrap();
    set.user.as_mut().unwrap().revision = 5;
    let raw = set_text(&set, &[]).unwrap();
    MemoryStore { current: Some(raw.clone()), snapshot_map: [(5, raw)].into(), ..Default::default() }
}

#[test]
fn browser_autosave_writes_dirty_drawings_through_the_sink() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage::default()));
    let mut app = browser(Vec::new(), storage.clone());
    line(&mut app);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    assert!(storage.borrow().writes.is_empty());
    frame(&mut app, &ctx, 61.0);
    let state = app.session.state().unwrap();
    let name = std::path::Path::new("idb").join(autosave::file_name(&state.title, "123-ab", state.uid)).to_string_lossy().to_string();
    let storage_ref = storage.borrow();
    let (actual, title, bytes) = storage_ref.writes.first().unwrap();
    assert_eq!(actual, &name);
    assert_eq!(title, &state.title);
    assert_eq!(cadcraft_io::read(bytes, actual).unwrap().model.len(), 1);
    assert_eq!(storage_ref.writes.len(), 1);
    assert_eq!(storage_ref.beats.len(), 2);
    drop(storage_ref);
    assert!(matches!(DrawingSave::from_app(&app), DrawingSave::AutosavedLocal { .. }));
    assert!(app.has_unsaved_drawings());
    let state = app.session.state_mut().unwrap();
    state.saved = state.doc.clone();
    frame(&mut app, &ctx, 62.0);
    assert_eq!(storage.borrow().deletes, [name]);
}

#[test]
fn browser_autosave_skips_oversize_drawings() {
    let _guard = setup();
    assert_eq!(MAX_WEB_AUTOSAVE_BYTES, 64 << 20);
    let storage = Rc::new(RefCell::new(Storage { limit: 1, ..Default::default() }));
    let mut app = browser(Vec::new(), storage.clone());
    line(&mut app);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    frame(&mut app, &ctx, 61.0);
    assert!(storage.borrow().writes.is_empty());
    assert_eq!(app.drawing_autosave_error(), Some("Drawing too large to autosave in the browser"));
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Dirty);
}

#[test]
fn crash_recovery_tc13_keeps_drawing_and_workspace_apart() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage::default()));
    let first = recovery(&storage, 1);
    let second = recovery(&storage, 2);
    let mut app = browser(vec![first.clone()], storage.clone());
    app.attach_profile_store(Box::new(workspace()));
    assert_eq!(app.recovery.items.len(), 1);
    let status = app.run("ui.recovery.status", json!({})).unwrap();
    assert_eq!(status["drawings"].as_array().unwrap().len(), 1);
    assert_eq!(status["drawings"][0]["name"], first.path);
    assert_eq!(status["workspace"], json!({"source": "current", "revision": 5}));
    assert!(!app.ui.show_toolbar);
    let layout = app.ws.user.clone();
    let current = app.ws.store.as_ref().unwrap().read_current();
    app.recover_item(0).unwrap();
    assert_eq!(app.recovery.items.len(), 1, "the item stays until fetching and recovery succeed");
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    assert!(app.recovery.items.is_empty());
    let recovered = app.session.state().unwrap();
    assert_eq!(recovered.title, first.drawing);
    assert!(recovered.path.is_none() && recovered.is_dirty());
    assert_eq!(recovered.doc.model.len(), 1);
    assert!(app.autosave.as_ref().unwrap().files().contains(&first.path));
    assert!(storage.borrow().files.contains_key(&first.path));
    assert_eq!(app.ws.user, layout);
    assert_eq!(app.ws.store.as_ref().unwrap().read_current(), current);
    assert!(!app.ui.show_toolbar);
    app.recovery.items.push(second.clone());
    app.run("ui.workspace.reset", json!({})).unwrap();
    assert_eq!(app.recovery.items, [second]);
    assert!(app.session.state().unwrap().is_dirty());
    app.discard_item(0);
    assert_eq!(storage.borrow().deletes.len(), 1);
}

#[test]
fn recovery_after_snapshot_restore_reports_both() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage::default()));
    let item = recovery(&storage, 1);
    let mut store = workspace();
    store.current = Some("damaged".into());
    let mut app = browser(vec![item], storage);
    app.attach_profile_store(Box::new(store));
    let status = app.run("ui.recovery.status", json!({})).unwrap();
    assert_eq!(status["workspace"], json!({"source": "snapshot", "revision": 5}));
    assert_eq!(status["drawings"].as_array().unwrap().len(), 1);
    assert!(!app.ui.show_toolbar);
}

#[test]
fn browser_autosave_reports_only_durable_current_bytes() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage { auto_complete: false, ..Default::default() }));
    let mut app = browser(Vec::new(), storage.clone());
    line(&mut app);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    frame(&mut app, &ctx, 61.0);
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::SavingLocal);
    let name = storage.borrow().writes.first().unwrap().0.clone();
    storage.borrow_mut().durable.insert(name, 1_800_000_000_000.0);
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::AutosavedLocal { at_ms: 1_800_000_000_000.0 });
    line(&mut app);
    assert_eq!(DrawingSave::from_app(&app), DrawingSave::Dirty);
}

#[test]
fn browser_autosave_urgent_respects_savetime_and_retries_failed_writes() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage { fail_write: true, ..Default::default() }));
    let mut app = browser(Vec::new(), storage.clone());
    line(&mut app);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    app.browser_autosave_urgent(&ctx);
    assert_eq!(app.drawing_autosave_error(), Some("Browser storage is full"));
    storage.borrow_mut().fail_write = false;
    frame(&mut app, &ctx, 61.0);
    assert_eq!(storage.borrow().writes.len(), 1);
    assert!(app.drawing_autosave_error().is_none());
    line(&mut app);
    autosave::set_savetime(0);
    app.browser_autosave_urgent(&ctx);
    assert_eq!(storage.borrow().writes.len(), 1);
    autosave::set_savetime(1);
    app.browser_autosave_urgent(&ctx);
    assert_eq!(storage.borrow().writes.len(), 2);
    app.shutdown_autosave();
    assert!(storage.borrow().deletes.is_empty(), "browser shutdown retains recovery copies");
}

#[test]
fn failed_browser_recovery_keeps_the_item_for_retry() {
    let _guard = setup();
    let storage = Rc::new(RefCell::new(Storage::default()));
    let item = recovery(&storage, 1);
    storage.borrow_mut().files.insert(item.path.clone(), b"bad".to_vec());
    let mut app = browser(vec![item.clone()], storage.clone());
    app.recover_item(0).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, 0.0);
    assert_eq!(app.recovery.items, [item]);
    assert_eq!(app.session.docs.len(), 1);
    assert!(storage.borrow().deletes.is_empty());
}

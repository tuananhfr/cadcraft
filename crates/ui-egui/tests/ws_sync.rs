// The transport contract returns stale profiles by value.
#![allow(clippy::result_large_err)]

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use cadcraft_engine::Session;
use cadcraft_ui_egui::wsprofile::SaveStatus;
use cadcraft_ui_egui::wsprofile::conflict::{Resolution, conflicts, show_conflict_dialog};
use cadcraft_ui_egui::wsprofile::presets::preset;
use cadcraft_ui_egui::wsprofile::schema::{Dock, OwnerType, ToolbarState, WorkspaceProfile};
use cadcraft_ui_egui::wsprofile::startup::parse_set;
use cadcraft_ui_egui::wsprofile::store::{MemoryStore, ProfileSet, ProfileStore};
use cadcraft_ui_egui::wsprofile::sync::{MemoryRemote, ProfileRemote, RemoteError};
use cadcraft_ui_egui::{CadApp, Services};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

fn base() -> WorkspaceProfile {
    let mut profile = preset("SIMPLE").unwrap();
    profile.profile_id = "user.shared".into();
    profile.name = "Workspace".into();
    profile.base_id = Some("SIMPLE".into());
    profile.owner_type = OwnerType::User;
    profile.etag = Some("\"initial:1\"".into());
    profile.layout.toolbars.push(ToolbarState { id: "draw".into(), label: "Draw".into(), commands: vec!["line".into()], ..Default::default() });
    profile
}

#[derive(Clone)]
struct SharedStore(Rc<RefCell<MemoryStore>>);

impl SharedStore {
    fn seeded(profile: &WorkspaceProfile) -> Self {
        let set = ProfileSet { user: Some(profile.clone()), ..Default::default() };
        let text = serde_json::to_string(&set).unwrap();
        Self(Rc::new(RefCell::new(MemoryStore { current: Some(text.clone()), snapshot_map: BTreeMap::from([(1, text)]), ..Default::default() })))
    }

    fn current(&self) -> ProfileSet {
        parse_set(&self.read_current().unwrap()).unwrap()
    }
}

impl ProfileStore for SharedStore {
    fn read_current(&self) -> Option<String> {
        self.0.borrow().read_current()
    }
    fn write_current(&mut self, text: &str) -> Result<(), String> {
        self.0.borrow_mut().write_current(text)
    }
    fn push_snapshot(&mut self, revision: u64, text: &str) -> Result<(), String> {
        self.0.borrow_mut().push_snapshot(revision, text)
    }
    fn snapshots(&self) -> Vec<u64> {
        self.0.borrow().snapshots()
    }
    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.0.borrow().read_snapshot(revision)
    }
}

#[derive(Clone)]
struct SharedRemote {
    inner: Rc<RefCell<MemoryRemote>>,
    calls: Rc<Cell<usize>>,
    stale: Rc<Cell<usize>>,
}

impl SharedRemote {
    fn seeded(profile: &WorkspaceProfile) -> Self {
        Self {
            inner: Rc::new(RefCell::new(MemoryRemote {
                profiles: BTreeMap::from([(profile.profile_id.clone(), profile.clone())]),
                online: true,
                forbidden: BTreeSet::new(),
            })),
            calls: Rc::new(Cell::new(0)),
            stale: Rc::new(Cell::new(0)),
        }
    }

    fn profile(&self) -> WorkspaceProfile {
        self.inner.borrow().profiles.get("user.shared").unwrap().clone()
    }
}

impl ProfileRemote for SharedRemote {
    fn get(&mut self, id: &str) -> Result<WorkspaceProfile, RemoteError> {
        self.calls.set(self.calls.get() + 1);
        self.inner.borrow_mut().get(id)
    }
    fn patch(&mut self, id: &str, if_match: &str, patch: &Value) -> Result<WorkspaceProfile, RemoteError> {
        self.calls.set(self.calls.get() + 1);
        let result = self.inner.borrow_mut().patch(id, if_match, patch);
        if matches!(result, Err(RemoteError::Stale { .. })) {
            self.stale.set(self.stale.get() + 1);
        }
        result
    }
}

fn app(store: &SharedStore, remote: &SharedRemote) -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store.clone()));
    app.attach_remote(Box::new(remote.clone()));
    app
}

fn set(app: &mut CadApp, pointer: &str, value: Value) {
    app.run("ui.workspace.set", json!({"pointer": pointer, "value": value})).unwrap();
}

fn commit(app: &mut CadApp) {
    app.commit_workspace_now().unwrap();
    app.sync_now();
}

fn draw(profile: &WorkspaceProfile) -> Dock {
    profile.layout.toolbars.iter().find(|toolbar| toolbar.id == "draw").unwrap().dock
}

#[test]
fn offline_edit_then_online_syncs_ux05() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    remote.inner.borrow_mut().online = false;
    let mut first = app(&store, &remote);
    set(&mut first, "/toolbars/draw/dock", json!("left"));
    set(&mut first, "/command_line/lines", json!(5));
    set(&mut first, "/status_bar/visible", json!(false));
    commit(&mut first);
    assert_eq!(first.ws.status, SaveStatus::Offline);
    let local = store.current().user.unwrap();
    assert_eq!(draw(&local), Dock::Left);
    assert_eq!(local.layout.command_line.lines, 5);
    assert!(!local.layout.status_bar.visible);
    assert_eq!(remote.profile().revision, 1);
    drop(first);

    remote.inner.borrow_mut().online = true;
    let mut restarted = app(&store, &remote);
    restarted.sync_now();
    assert_eq!(restarted.ws.status, SaveStatus::Synced);
    let server = remote.profile();
    assert!(server.revision > 1);
    assert_eq!(draw(&server), Dock::Left);
    assert_eq!(server.layout.command_line.lines, 5);
    assert!(!server.layout.status_bar.visible);
    let mut restarted_again = app(&store, &remote);
    restarted_again.sync_now();
    assert_eq!(restarted_again.ws.status, SaveStatus::Synced);
    assert_eq!(remote.profile().revision, server.revision, "an acknowledged revision is not uploaded twice");
}

#[test]
fn two_machines_same_field_conflict_ux06() {
    let base = base();
    let remote = SharedRemote::seeded(&base);
    let store_a = SharedStore::seeded(&base);
    let store_b = SharedStore::seeded(&base);
    let mut a = app(&store_a, &remote);
    let mut b = app(&store_b, &remote);
    set(&mut a, "/toolbars/draw/dock", json!("left"));
    commit(&mut a);
    set(&mut b, "/toolbars/draw/dock", json!("right"));
    commit(&mut b);
    assert_eq!(a.ws.status, SaveStatus::Synced);
    assert_eq!(b.ws.status, SaveStatus::Conflict);
    assert!(remote.stale.get() > 0);
    let fields = conflicts(
        &serde_json::to_value(&base.layout).unwrap(),
        &serde_json::to_value(&b.ws.user.layout).unwrap(),
        &serde_json::to_value(&remote.profile().layout).unwrap(),
    );
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].pointer, "/toolbars/draw/dock");
    assert_eq!(fields[0].mine, json!("right"));
    assert_eq!(fields[0].theirs, json!("left"));
    let server = remote.profile();
    b.resolve_conflict(Resolution::KeepBoth);
    assert_eq!(b.ws.status, SaveStatus::Synced);
    assert_eq!(remote.profile(), server, "KeepBoth does not overwrite the server revision");
    assert_eq!(draw(&b.ws.user), Dock::Left);
    let copy = b.ws.profiles.iter().find(|profile| profile.name == "Workspace (my copy)").unwrap();
    assert_ne!(copy.profile_id, server.profile_id);
    assert!(copy.etag.is_none());
    assert_eq!(draw(copy), Dock::Right);
    let saved = store_b.current();
    assert_eq!(draw(&saved.user.unwrap()), Dock::Left);
    assert_eq!(draw(&saved.profiles[0]), Dock::Right);
    let snapshots: Vec<_> = store_b.snapshots().iter().map(|rev| parse_set(&store_b.read_snapshot(*rev).unwrap()).unwrap()).collect();
    assert!(snapshots.iter().any(|set| set.user.as_ref().is_some_and(|profile| draw(profile) == Dock::Top)));
    assert!(snapshots.iter().any(|set| set.user.as_ref().is_some_and(|profile| draw(profile) == Dock::Right)));
    let restarted = app(&store_b, &remote);
    assert_eq!(draw(&restarted.ws.user), Dock::Left);
    assert_eq!(draw(&restarted.ws.profiles[0]), Dock::Right);
}

#[test]
fn different_fields_merge_silently() {
    let base = base();
    let remote = SharedRemote::seeded(&base);
    let mut a = app(&SharedStore::seeded(&base), &remote);
    let mut b = app(&SharedStore::seeded(&base), &remote);
    set(&mut a, "/toolbars/draw/dock", json!("left"));
    commit(&mut a);
    set(&mut b, "/command_line/lines", json!(7));
    commit(&mut b);
    assert_eq!(b.ws.status, SaveStatus::Synced);
    assert!(b.ws.sync.pending.is_none());
    assert_eq!(draw(&b.ws.user), Dock::Left);
    assert_eq!(b.ws.user.layout.command_line.lines, 7);
    assert_eq!(remote.profile().revision, 3);
    assert_eq!(remote.profile().layout.command_line.lines, 7);
    assert_eq!(draw(&remote.profile()), Dock::Left);
}

#[test]
fn forbidden_is_reported_not_retried_forever() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    remote.inner.borrow_mut().forbidden.insert(base.profile_id.clone());
    let mut app = app(&store, &remote);
    set(&mut app, "/command_line/lines", json!(7));
    commit(&mut app);
    assert_eq!(app.ws.status, SaveStatus::Offline);
    assert!(app.ws.message.as_ref().unwrap().contains("forbidden"));
    assert_eq!(store.current().user.unwrap().layout.command_line.lines, 7);
    let calls = remote.calls.get();
    for _ in 0..20 {
        app.sync_now();
    }
    assert_eq!(remote.calls.get(), calls);
    assert_eq!(remote.profile().revision, 1);
    remote.inner.borrow_mut().forbidden.clear();
    app.attach_remote(Box::new(remote.clone()));
    app.sync_now();
    assert_eq!(app.ws.status, SaveStatus::Synced);
    assert_eq!(remote.profile().layout.command_line.lines, 7);
}

#[test]
fn keep_mine_and_use_server_are_explicit() {
    for resolution in [Resolution::KeepMine, Resolution::UseServer] {
        let base = base();
        let remote = SharedRemote::seeded(&base);
        let mut a = app(&SharedStore::seeded(&base), &remote);
        let store_b = SharedStore::seeded(&base);
        let mut b = app(&store_b, &remote);
        set(&mut a, "/toolbars/draw/dock", json!("left"));
        set(&mut a, "/command_line/lines", json!(5));
        commit(&mut a);
        set(&mut b, "/toolbars/draw/dock", json!("right"));
        commit(&mut b);
        assert_eq!(b.ws.status, SaveStatus::Conflict);
        b.resolve_conflict(resolution);
        let expected = if resolution == Resolution::KeepMine { Dock::Right } else { Dock::Left };
        assert_eq!(b.ws.status, SaveStatus::Synced);
        assert_eq!(draw(&b.ws.user), expected);
        assert_eq!(draw(&remote.profile()), expected);
        assert_eq!(b.ws.user.layout.command_line.lines, 5, "server-only changes survive either choice");
        assert!(b.ws.sync.pending.is_none());
    }
}

#[test]
fn offline_restart_preserves_base_for_later_conflicts() {
    let base = base();
    let remote = SharedRemote::seeded(&base);
    let store_b = SharedStore::seeded(&base);
    let mut a = app(&SharedStore::seeded(&base), &remote);
    let mut b = app(&store_b, &remote);
    remote.inner.borrow_mut().online = false;
    set(&mut b, "/toolbars/draw/dock", json!("right"));
    commit(&mut b);
    drop(b);
    remote.inner.borrow_mut().online = true;
    set(&mut a, "/toolbars/draw/dock", json!("left"));
    commit(&mut a);
    let mut b = app(&store_b, &remote);
    b.sync_now();
    assert_eq!(b.ws.status, SaveStatus::Conflict);
    assert_eq!(b.ws.sync.pending.as_ref().unwrap().fields[0].pointer, "/toolbars/draw/dock");
    assert_eq!(draw(&b.ws.user), Dock::Right);
    assert_eq!(draw(&remote.profile()), Dock::Left);
}

#[test]
fn sync_waits_for_local_commit_and_failed_storage() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    let mut app = app(&store, &remote);
    set(&mut app, "/command_line/lines", json!(7));
    app.sync_now();
    assert_eq!(remote.calls.get(), 0);
    assert_eq!(app.ws.status, SaveStatus::Unsaved);
    store.0.borrow_mut().fail_writes = true;
    assert!(app.commit_workspace_now().is_err());
    app.sync_now();
    assert_eq!(remote.calls.get(), 0);
    assert_eq!(remote.profile().layout.command_line.lines, 3);
    assert_eq!(app.ws.user.layout.command_line.lines, 7);
}

#[test]
fn memory_remote_checks_etag_permissions_and_revision_overflow() {
    let base = base();
    let mut remote = MemoryRemote { profiles: BTreeMap::from([(base.profile_id.clone(), base.clone())]), online: true, ..Default::default() };
    assert!(matches!(remote.get("missing"), Err(RemoteError::NotFound)));
    assert!(matches!(remote.patch(&base.profile_id, "wrong", &json!({"layout": {"command_line": {"lines": 7}}})), Err(RemoteError::Stale { .. })));
    assert_eq!(remote.get(&base.profile_id).unwrap(), base);
    let next = remote.patch(&base.profile_id, base.etag.as_deref().unwrap(), &json!({"layout": {"command_line": {"lines": 7}}})).unwrap();
    assert_eq!(next.revision, 2);
    assert_ne!(next.etag, base.etag);
    assert_eq!(next.layout.command_line.lines, 7);
    remote.profiles.get_mut(&base.profile_id).unwrap().revision = u64::MAX;
    assert!(matches!(remote.patch(&base.profile_id, next.etag.as_deref().unwrap(), &json!({})), Err(RemoteError::Forbidden(_))));
    assert_eq!(remote.profiles.get(&base.profile_id).unwrap().revision, u64::MAX);
    remote.profiles.get_mut(&base.profile_id).unwrap().etag = None;
    assert!(remote.get(&base.profile_id).unwrap().etag.is_some());
}

#[test]
fn conflicts_use_ids_escape_pointers_and_distinguish_deletions() {
    let base = json!({"toolbars": [{"id": "draw/a~b", "dock": "top"}], "extra": {"one": 1}});
    let mine = json!({"toolbars": [{"id": "draw/a~b", "dock": "left"}], "extra": {}});
    let theirs = json!({"toolbars": [{"id": "draw/a~b", "dock": "right"}], "extra": {"one": 2}});
    let fields = conflicts(&base, &mine, &theirs);
    assert_eq!(fields.len(), 2);
    assert!(fields.iter().any(|field| field.pointer == "/toolbars/draw~1a~0b/dock"));
    assert!(fields.iter().any(|field| field.pointer == "/extra/one" && field.mine.is_null() && field.theirs == json!(2)));
    assert!(conflicts(&base, &mine, &mine).is_empty());
    assert!(conflicts(&base, &mine, &base).is_empty());
    let base = json!({"toolbars": [{"id": "draw"}]});
    let mine = json!({"toolbars": [{"id": "draw"}, {"id": "mine"}]});
    let theirs = json!({"toolbars": [{"id": "draw"}, {"id": "theirs"}]});
    assert!(conflicts(&base, &mine, &theirs).is_empty());
}

#[test]
fn remote_attachment_keeps_base_before_first_edit() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    let _app = app(&store, &remote);
    let set = store.current();
    let stored: WorkspaceProfile = serde_json::from_value(set.extra["workspace_sync_bases"][&base.profile_id].clone()).unwrap();
    assert_eq!(stored, base);
    assert_eq!(remote.calls.get(), 0, "attaching only persists the local baseline");
}

#[test]
fn use_server_and_keep_both_fetch_the_latest_server() {
    for resolution in [Resolution::UseServer, Resolution::KeepBoth] {
        let base = base();
        let remote = SharedRemote::seeded(&base);
        let mut a = app(&SharedStore::seeded(&base), &remote);
        let mut b = app(&SharedStore::seeded(&base), &remote);
        set(&mut a, "/toolbars/draw/dock", json!("left"));
        commit(&mut a);
        set(&mut b, "/toolbars/draw/dock", json!("right"));
        commit(&mut b);
        assert_eq!(b.ws.status, SaveStatus::Conflict);
        set(&mut a, "/command_line/lines", json!(9));
        commit(&mut a);
        let latest = remote.profile();
        b.resolve_conflict(resolution);
        assert_eq!(b.ws.status, SaveStatus::Synced);
        assert_eq!(b.ws.user.layout.command_line.lines, 9);
        assert_eq!(draw(&b.ws.user), Dock::Left);
        assert_eq!(remote.profile(), latest);
        if resolution == Resolution::KeepBoth {
            assert_eq!(draw(&b.ws.profiles[0]), Dock::Right);
            assert_eq!(b.ws.profiles[0].layout.command_line.lines, 3);
        }
    }
}

#[test]
fn conflict_dialog_lists_fields_and_resolves_keep_both() {
    let base = base();
    let remote = SharedRemote::seeded(&base);
    let mut a = app(&SharedStore::seeded(&base), &remote);
    let mut b = app(&SharedStore::seeded(&base), &remote);
    set(&mut a, "/toolbars/draw/dock", json!("left"));
    commit(&mut a);
    set(&mut b, "/toolbars/draw/dock", json!("right"));
    commit(&mut b);
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut CadApp| show_conflict_dialog(ui.ctx(), app), b);
    h.run_steps(2);
    h.get_by_label("/toolbars/draw/dock");
    h.get_by_label("Keep mine");
    h.get_by_label("Use server");
    h.get_by_label("Keep both").click();
    h.run_steps(2);
    assert_eq!(h.state().ws.status, SaveStatus::Synced);
    assert_eq!(draw(&h.state().ws.user), Dock::Left);
    assert_eq!(draw(&h.state().ws.profiles[0]), Dock::Right);
}

struct FailAcknowledgement {
    remote: SharedRemote,
    store: SharedStore,
}

impl ProfileRemote for FailAcknowledgement {
    fn get(&mut self, id: &str) -> Result<WorkspaceProfile, RemoteError> {
        self.remote.get(id)
    }
    fn patch(&mut self, id: &str, if_match: &str, patch: &Value) -> Result<WorkspaceProfile, RemoteError> {
        let result = self.remote.patch(id, if_match, patch);
        if result.is_ok() {
            self.store.0.borrow_mut().fail_writes = true;
        }
        result
    }
}

#[test]
fn failed_local_acknowledgement_keeps_edits_and_can_retry() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    let mut app = app(&store, &remote);
    app.attach_remote(Box::new(FailAcknowledgement { remote: remote.clone(), store: store.clone() }));
    set(&mut app, "/command_line/lines", json!(7));
    app.commit_workspace_now().unwrap();
    app.sync_now();
    assert!(matches!(app.ws.status, SaveStatus::Failed(_)));
    assert_eq!(app.ws.user.layout.command_line.lines, 7);
    assert_eq!(store.current().user.unwrap().layout.command_line.lines, 7);
    assert_eq!(remote.profile().revision, 2);
    assert!(app.ws.autosave.pending());
    store.0.borrow_mut().fail_writes = false;
    commit(&mut app);
    assert_eq!(app.ws.status, SaveStatus::Synced);
    assert_eq!(remote.profile().revision, 2, "retrying an acknowledgement does not create another remote revision");
}

#[test]
fn sync_without_a_remote_preserves_local_status() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.skip_first_run_picker = true;
    app.attach_profile_store(Box::new(store.clone()));
    set(&mut app, "/command_line/lines", json!(5));
    commit(&mut app);
    assert_eq!(app.ws.status, SaveStatus::SavedLocally);
    assert_eq!(store.current().user.unwrap().layout.command_line.lines, 5);
}

struct RacingRemote {
    server: WorkspaceProfile,
    patches: Rc<Cell<usize>>,
}

impl ProfileRemote for RacingRemote {
    fn get(&mut self, _id: &str) -> Result<WorkspaceProfile, RemoteError> {
        Ok(self.server.clone())
    }
    fn patch(&mut self, _id: &str, _if_match: &str, _patch: &Value) -> Result<WorkspaceProfile, RemoteError> {
        self.patches.set(self.patches.get() + 1);
        self.server.revision += 1;
        self.server.etag = Some(format!("\"racing:{}\"", self.server.revision));
        self.server.layout.toolbars.iter_mut().find(|toolbar| toolbar.id == "draw").unwrap().dock = Dock::Left;
        Err(RemoteError::Stale { server: self.server.clone() })
    }
}

#[test]
fn repeated_stale_responses_have_a_bounded_retry() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    let mut app = app(&store, &remote);
    let patches = Rc::new(Cell::new(0));
    app.attach_remote(Box::new(RacingRemote { server: base, patches: patches.clone() }));
    set(&mut app, "/command_line/lines", json!(7));
    app.commit_workspace_now().unwrap();
    let after_commit = patches.get();
    if after_commit == 0 {
        app.sync_now();
    }
    assert_eq!(patches.get(), 2, "one stale retry is allowed per sync attempt");
    assert_eq!(app.ws.status, SaveStatus::Offline);
    assert_eq!(app.ws.user.layout.command_line.lines, 7);
    assert_eq!(store.current().user.unwrap().layout.command_line.lines, 7);
}

#[test]
fn ui_scale_is_not_synced() {
    let base = base();
    let store = SharedStore::seeded(&base);
    let remote = SharedRemote::seeded(&base);
    let mut first = app(&store, &remote);
    set(&mut first, "/ui_scale", json!(1.5));
    commit(&mut first);
    assert_eq!(first.ws.status, SaveStatus::Synced);
    assert_eq!(first.ws.device.ui_scale, 1.5);
    assert_eq!(store.current().device.unwrap().ui_scale, 1.5);
    assert!(serde_json::to_value(remote.profile()).unwrap().get("ui_scale").is_none());
    assert_eq!(remote.profile().revision, base.revision, "a device edit sends no profile patch");
    let second = app(&SharedStore::seeded(&base), &remote);
    assert_eq!(second.ws.device.ui_scale, 1.0);
    let restarted = app(&store, &remote);
    assert_eq!(restarted.ws.device.ui_scale, 1.5, "sync baseline persistence keeps local geometry");
}

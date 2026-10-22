//! A synchronous workspace mirror whose writes become durable in the browser's background queue.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use cadcraft_ui_egui::wsprofile::startup::{parse_set, set_text, stored_revision};
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, Commit, Durability, MAX_SET_BYTES, ProfileStore, TAB_CONFLICT_MESSAGE};

use crate::profile_store::{
    CURRENT_KEY, DAMAGED_INDEX_KEY, INDEX_KEY, KeyValue, damaged_key, parse_index, remember_created, snapshot_key, stamp_clock,
};

pub const MAX_PRELOAD_RECORDS: usize = 10_000;
const MIGRATED: &str = "migrated_from_local_storage";
const BACKUP: &str = "backup:1.0.0";
const LOCAL_BACKUP: &str = "cadcraft.ws.backup:1.0.0";
const MAX_QUEUED_WRITES: usize = 64;
pub const QUOTA_MESSAGE: &str = "Browser storage is full";

#[derive(Clone, Debug, PartialEq)]
pub enum IdbError {
    Quota,
    Unavailable(String),
    Aborted(String),
    Conflict { stored: u64 },
}

impl IdbError {
    pub fn message(&self) -> String {
        match self {
            Self::Quota => QUOTA_MESSAGE.into(),
            Self::Unavailable(message) | Self::Aborted(message) => message.clone(),
            Self::Conflict { .. } => TAB_CONFLICT_MESSAGE.into(),
        }
    }
}

#[derive(Default)]
struct PendingState {
    result: Option<Result<(), IdbError>>,
    waker: Option<std::task::Waker>,
    observed_current: Option<String>,
}

#[derive(Clone, Default)]
pub struct Pending(Rc<RefCell<PendingState>>);

impl Pending {
    pub fn result(&self) -> Option<Result<(), IdbError>> {
        self.0.try_borrow().ok().and_then(|state| state.result.clone())
    }

    pub fn observe_current(&self, json: String) {
        if let Ok(mut state) = self.0.try_borrow_mut() {
            state.observed_current = Some(json);
        }
    }

    fn observed_current(&self) -> Option<String> {
        self.0.try_borrow().ok().and_then(|state| state.observed_current.clone())
    }

    pub fn complete(&self, result: Result<(), IdbError>) {
        let wake = if let Ok(mut state) = self.0.try_borrow_mut() {
            state.result = Some(result);
            state.waker.take()
        } else {
            None
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn wait(&self) -> Result<(), IdbError> {
        std::future::poll_fn(|cx| {
            let Ok(mut state) = self.0.try_borrow_mut() else {
                return std::task::Poll::Ready(Err(IdbError::Unavailable("workspace write is busy".into())));
            };
            if let Some(result) = &state.result {
                std::task::Poll::Ready(result.clone())
            } else {
                state.waker = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        })
        .await
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mirror {
    pub current: Option<String>,
    pub snapshots: BTreeMap<u64, String>,
    pub damaged: BTreeMap<u64, Vec<u8>>,
    pub meta: BTreeMap<String, String>,
}

impl Mirror {
    pub fn needs_migration(&self) -> bool {
        self.current.is_none() && self.snapshots.is_empty() && self.damaged.is_empty() && !self.meta.contains_key(MIGRATED)
    }

    pub fn preload(&mut self, store: &str, records: impl IntoIterator<Item = (String, Vec<u8>)>) {
        for (index, (key, bytes)) in records.into_iter().enumerate() {
            if index >= MAX_PRELOAD_RECORDS {
                log::warn!("workspace: IndexedDB preload capped at {MAX_PRELOAD_RECORDS} records");
                break;
            }
            if bytes.len() > MAX_SET_BYTES || key.len() > MAX_SET_BYTES {
                log::warn!("workspace: oversized IndexedDB record ignored");
                continue;
            }
            if store == "damaged" {
                if let Ok(id) = key.parse() {
                    self.damaged.insert(id, bytes);
                }
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else { continue };
            match store {
                "workspace" if key == "current" => self.current = Some(text),
                "snapshots" => {
                    if let Ok(revision) = key.parse() {
                        self.snapshots.insert(revision, text);
                    }
                }
                "meta" => {
                    self.meta.insert(key, text);
                }
                _ => {}
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OwnedCommit {
    pub expected: u64,
    pub revision: u64,
    pub json: String,
    pub prune: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Commit(OwnedCommit),
    Current(String),
    Snapshot(u64, String),
    KeepDamaged(u64, Vec<u8>),
    PutMeta(String, String),
    Evict(Vec<u64>),
}

pub trait Backend {
    fn submit(&mut self, op: Op) -> Pending;
    fn broadcast(&mut self, _revision: u64, _json: &str) {}
}

struct Job {
    op: Op,
    before: Mirror,
    pending: Option<Pending>,
    retry: bool,
    evicting: bool,
    evicted: Vec<u64>,
}

pub struct IdbProfileStore<B: Backend> {
    mirror: Mirror,
    backend: B,
    queue: VecDeque<Job>,
    outcome: Durability,
    external: Rc<Cell<bool>>,
    incoming: Rc<RefCell<Option<(u64, String)>>>,
    now_ms: fn() -> f64,
    created: BTreeMap<String, String>,
    persisted: bool,
    required: Vec<Op>,
}

impl<B: Backend> IdbProfileStore<B> {
    pub fn new(mirror: Mirror, backend: B, now_ms: fn() -> f64, persisted: bool) -> Self {
        let mut created = BTreeMap::new();
        if let Some(raw) = &mirror.current {
            remember_created(raw, &mut created);
        }
        Self {
            mirror,
            backend,
            queue: VecDeque::new(),
            outcome: Durability::Committed,
            external: Rc::new(Cell::new(false)),
            incoming: Rc::new(RefCell::new(None)),
            now_ms,
            created,
            persisted,
            required: Vec::new(),
        }
    }

    pub fn on_broadcast(&mut self, revision: u64, json: String) {
        if json.len() > MAX_SET_BYTES || stored_revision(&json) != Some(revision) || parse_set(&json).is_err() {
            return;
        }
        if !self.queue.is_empty() {
            if let Ok(mut incoming) = self.incoming.try_borrow_mut()
                && incoming.as_ref().is_none_or(|(old, _)| revision > *old)
            {
                *incoming = Some((revision, json));
            }
            return;
        }
        let known = self.mirror.current.as_deref().and_then(stored_revision).unwrap_or(0);
        if revision > known {
            remember_created(&json, &mut self.created);
            self.mirror.snapshots.insert(revision, json.clone());
            self.mirror.current = Some(json);
            self.external.set(true);
        }
    }

    fn receive(&mut self) {
        if self.queue.is_empty() {
            let incoming = self.incoming.try_borrow_mut().ok().and_then(|mut slot| slot.take());
            if let Some((revision, json)) = incoming {
                self.on_broadcast(revision, json);
            }
        }
    }

    fn stamped(&mut self, json: &str) -> Result<String, String> {
        within(json.as_bytes())?;
        let text = stamp_clock(json, &cadcraft_ui_egui::wsprofile::rfc3339((self.now_ms)()), &mut self.created);
        within(text.as_bytes())?;
        Ok(text)
    }

    fn enqueue(&mut self, op: Op) -> Result<(), String> {
        if self.queue.len() >= MAX_QUEUED_WRITES {
            return Err("workspace write queue is full".into());
        }
        let before = self.mirror.clone();
        match &op {
            Op::Commit(c) => {
                self.mirror.current = Some(c.json.clone());
                self.mirror.snapshots.insert(c.revision, c.json.clone());
                for revision in &c.prune {
                    self.mirror.snapshots.remove(revision);
                }
            }
            Op::Current(json) => self.mirror.current = Some(json.clone()),
            Op::Snapshot(revision, json) => {
                self.mirror.snapshots.insert(*revision, json.clone());
            }
            Op::KeepDamaged(id, bytes) => {
                self.mirror.damaged.insert(*id, bytes.clone());
            }
            Op::PutMeta(key, value) => {
                self.mirror.meta.insert(key.clone(), value.clone());
            }
            Op::Evict(revisions) => {
                for revision in revisions {
                    self.mirror.snapshots.remove(revision);
                }
            }
        }
        let pending = self.queue.is_empty().then(|| self.backend.submit(op.clone()));
        self.queue.push_back(Job { op, before, pending, retry: false, evicting: false, evicted: Vec::new() });
        self.outcome = Durability::Pending;
        Ok(())
    }

    fn protect(&mut self, op: Op) -> Result<(), String> {
        let known = self.required.contains(&op);
        if !known && self.required.len() >= MAX_QUEUED_WRITES {
            return Err("too many pending workspace backups".into());
        }
        self.enqueue(op.clone())?;
        if !known {
            self.required.push(op);
        }
        Ok(())
    }

    fn poll(&mut self) -> Durability {
        for _ in 0..MAX_QUEUED_WRITES {
            let Some(mut job) = self.queue.pop_front() else {
                self.receive();
                return self.outcome.clone();
            };
            let pending = job.pending.get_or_insert_with(|| self.backend.submit(job.op.clone()));
            let Some(result) = pending.result() else {
                self.queue.push_front(job);
                return Durability::Pending;
            };
            let observed = pending.observed_current();
            match result {
                Ok(()) if job.evicting => {
                    for revision in &job.evicted {
                        job.before.snapshots.remove(revision);
                        self.mirror.snapshots.remove(revision);
                        for queued in &mut self.queue {
                            queued.before.snapshots.remove(revision);
                        }
                    }
                    job.evicting = false;
                    job.pending = Some(self.backend.submit(job.op.clone()));
                    self.queue.push_front(job);
                }
                Ok(()) => {
                    self.required.retain(|op| op != &job.op);
                    if let Op::Commit(c) = &job.op {
                        self.backend.broadcast(c.revision, &c.json);
                    }
                    self.outcome = if self.queue.is_empty() { Durability::Committed } else { Durability::Pending };
                }
                Err(IdbError::Quota) if matches!(job.op, Op::Commit(_)) && !job.retry => {
                    // Only durable, readable history can protect a failed replacement.
                    let newest = job.before.snapshots.iter().rev().find(|(revision, json)| **revision != CORRUPT_SNAPSHOT && parse_set(json).is_ok());
                    let keep = newest
                        .map(|(revision, _)| *revision)
                        .or_else(|| job.before.snapshots.keys().rev().copied().find(|revision| *revision != CORRUPT_SNAPSHOT));
                    let old: Vec<u64> = job.before.snapshots.keys().copied().filter(|r| *r != CORRUPT_SNAPSHOT && Some(*r) != keep).collect();
                    job.retry = true;
                    if old.is_empty() {
                        job.pending = Some(self.backend.submit(job.op.clone()));
                    } else {
                        job.evicted = old.clone();
                        job.evicting = true;
                        job.pending = Some(self.backend.submit(Op::Evict(old)));
                    }
                    self.queue.push_front(job);
                }
                Err(error) => {
                    self.mirror = job.before;
                    if let Some(json) = observed {
                        self.mirror.current = Some(json);
                    }
                    self.queue.clear();
                    self.outcome = match error {
                        IdbError::Conflict { stored } => Durability::Conflict { stored },
                        error => Durability::Failed(error.message()),
                    };
                    self.receive();
                    return self.outcome.clone();
                }
            }
        }
        if self.queue.is_empty() { self.outcome.clone() } else { Durability::Pending }
    }
}

fn within(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_SET_BYTES { Err(format!("the workspace is larger than {MAX_SET_BYTES} bytes")) } else { Ok(()) }
}

impl<B: Backend> ProfileStore for IdbProfileStore<B> {
    fn keep_backup(&mut self, version: &str, raw: &str) -> Result<(), String> {
        within(raw.as_bytes())?;
        if version != "1.0.0" {
            return Err("unsupported backup version".into());
        }
        if self.mirror.meta.contains_key(BACKUP) {
            return Ok(());
        }
        let op = self
            .required
            .iter()
            .find(|op| matches!(op, Op::PutMeta(key, _) if key == BACKUP))
            .cloned()
            .unwrap_or_else(|| Op::PutMeta(BACKUP.into(), raw.into()));
        self.protect(op)
    }

    fn read_backup(&self, version: &str) -> Option<String> {
        (version == "1.0.0").then(|| self.mirror.meta.get(BACKUP).cloned()).flatten()
    }

    fn read_current(&self) -> Option<String> {
        self.mirror.current.clone()
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        let text = if self.read_backup("1.0.0").as_deref() == Some(json) {
            within(json.as_bytes())?;
            json.into()
        } else {
            self.stamped(json)?
        };
        self.enqueue(Op::Current(text))
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        let text = self.stamped(json)?;
        self.enqueue(Op::Snapshot(revision, text))?;
        let prune = cadcraft_ui_egui::wsprofile::store::expired_snapshots(&self.snapshots());
        if !prune.is_empty() {
            self.enqueue(Op::Evict(prune))?;
        }
        Ok(())
    }

    fn write_commit(&mut self, c: Commit<'_>) -> Result<(), String> {
        self.receive();
        let stored = self.mirror.current.as_deref().and_then(stored_revision).unwrap_or(0);
        if stored > c.expected {
            return Err(TAB_CONFLICT_MESSAGE.into());
        }
        for op in self.required.clone() {
            let missing = match &op {
                Op::PutMeta(key, _) => !self.mirror.meta.contains_key(key),
                Op::KeepDamaged(id, _) => !self.mirror.damaged.contains_key(id),
                _ => false,
            };
            if missing {
                self.enqueue(op)?;
            }
        }
        let text = self.stamped(c.json)?;
        if c.prune.len() > MAX_PRELOAD_RECORDS {
            return Err("too many workspace snapshots to prune".into());
        }
        let prune = c.prune.into_iter().filter(|r| *r != CORRUPT_SNAPSHOT && *r != c.revision).collect();
        self.enqueue(Op::Commit(OwnedCommit { expected: c.expected, revision: c.revision, json: text, prune }))
    }

    fn durability(&mut self) -> Durability {
        self.poll()
    }

    fn persisted_storage(&self) -> Option<bool> {
        Some(self.persisted)
    }

    fn snapshots(&self) -> Vec<u64> {
        self.mirror.snapshots.keys().copied().collect()
    }

    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.mirror.snapshots.get(&revision).cloned()
    }

    fn take_external_change(&mut self) -> bool {
        self.receive();
        self.external.replace(false)
    }

    fn keep_damaged(&mut self, raw: &[u8]) -> Result<String, String> {
        within(raw)?;
        if let Some((id, _)) = self.mirror.damaged.iter().find(|(_, bytes)| bytes.as_slice() == raw) {
            return Ok(damaged_key(*id));
        }
        if let Some(op) = self.required.iter().find(|op| matches!(op, Op::KeepDamaged(_, bytes) if bytes.as_slice() == raw)).cloned()
            && let Op::KeepDamaged(id, _) = &op
        {
            let name = damaged_key(*id);
            self.protect(op)?;
            return Ok(name);
        }
        let queued = self.required.iter().filter_map(|op| if let Op::KeepDamaged(id, _) = op { Some(*id) } else { None }).max().unwrap_or(0);
        let id = self.mirror.damaged.keys().next_back().copied().unwrap_or(0).max(queued).checked_add(1).ok_or("damaged copy index is full")?;
        self.protect(Op::KeepDamaged(id, raw.to_vec()))?;
        Ok(damaged_key(id))
    }
}

pub fn migrate_from_local_storage(kv: &dyn KeyValue) -> Option<Mirror> {
    let current = kv.get(CURRENT_KEY)?;
    if current.len() > MAX_SET_BYTES {
        return None;
    }
    let mut mirror = Mirror { current: Some(current), ..Mirror::default() };
    for revision in kv.get(INDEX_KEY).filter(|text| text.len() <= MAX_SET_BYTES).map(|text| parse_index(&text)).unwrap_or_default() {
        if let Some(text) = kv.get(&snapshot_key(revision)).filter(|text| text.len() <= MAX_SET_BYTES) {
            mirror.snapshots.insert(revision, text);
        }
    }
    if let Some(text) = kv.get(DAMAGED_INDEX_KEY).filter(|text| text.len() <= MAX_SET_BYTES)
        && let Ok(ids) = serde_json::from_str::<Vec<u64>>(&text)
    {
        for id in ids.into_iter().take(MAX_PRELOAD_RECORDS) {
            if let Some(raw) = kv.get(&damaged_key(id)).filter(|text| text.len() <= MAX_SET_BYTES) {
                mirror.damaged.insert(id, raw.into_bytes());
            }
        }
    }
    if let Some(raw) = kv.get(LOCAL_BACKUP).filter(|text| text.len() <= MAX_SET_BYTES) {
        mirror.meta.insert(BACKUP.into(), raw);
    }
    if let Some(raw) = mirror.current.as_deref()
        && let Ok(set) = parse_set(raw)
        && let Ok(converted) = set_text(&set, &[])
        && converted.len() <= MAX_SET_BYTES
    {
        let legacy = serde_json::from_str::<serde_json::Value>(raw).ok().is_some_and(|value| {
            let v1 =
                |profile: &serde_json::Value| {
                    profile.get("schema_version").or_else(|| profile.get("schemaVersion")).is_some_and(|version| {
                        version.as_u64() == Some(1) || version.as_str().is_some_and(|text| text.split('.').next() == Some("1"))
                    }) || profile.get("version").and_then(serde_json::Value::as_u64) == Some(1)
                };
            v1(&value)
                || value.get("user").is_some_and(v1)
                || value.get("profiles").and_then(serde_json::Value::as_array).is_some_and(|profiles| profiles.iter().take(20).any(v1))
        });
        if legacy {
            mirror.meta.entry(BACKUP.into()).or_insert_with(|| raw.to_string());
        }
        mirror.current = Some(converted);
    }
    mirror.meta.insert(MIGRATED.into(), "true".into());
    Some(mirror)
}

pub fn cleanup_local_storage(kv: &mut dyn KeyValue, mirror: &Mirror) {
    if !mirror.meta.contains_key(MIGRATED) {
        return;
    }
    // Keep concurrent localStorage edits and the old current key available to rollback builds.
    let snapshots_copied =
        kv.get(INDEX_KEY).filter(|text| text.len() <= MAX_SET_BYTES).and_then(|text| serde_json::from_str::<Vec<u64>>(&text).ok()).is_some_and(
            |ids| {
                ids.len() <= MAX_PRELOAD_RECORDS
                    && ids.iter().all(|id| mirror.snapshots.get(id).is_some_and(|text| kv.get(&snapshot_key(*id)).as_deref() == Some(text.as_str())))
            },
        );
    let damaged_copied = kv
        .get(DAMAGED_INDEX_KEY)
        .filter(|text| text.len() <= MAX_SET_BYTES)
        .and_then(|text| serde_json::from_str::<Vec<u64>>(&text).ok())
        .is_some_and(|ids| {
            ids.len() <= MAX_PRELOAD_RECORDS
                && ids.iter().all(|id| mirror.damaged.get(id).is_some_and(|raw| kv.get(&damaged_key(*id)).is_some_and(|text| text.as_bytes() == raw)))
        });
    for (revision, text) in &mirror.snapshots {
        let key = snapshot_key(*revision);
        if kv.get(&key).as_deref() == Some(text) {
            kv.remove(&key);
        }
    }
    for (id, raw) in &mirror.damaged {
        let key = damaged_key(*id);
        if kv.get(&key).is_some_and(|text| text.as_bytes() == raw) {
            kv.remove(&key);
        }
    }
    if snapshots_copied {
        kv.remove(INDEX_KEY);
    }
    if damaged_copied {
        kv.remove(DAMAGED_INDEX_KEY);
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use wasm_bindgen::{JsCast, JsValue, closure::Closure};

    pub struct IdbBackend {
        idb: crate::idb::IdbStore,
        channel: Option<web_sys::BroadcastChannel>,
        _listener: Option<Closure<dyn FnMut(web_sys::MessageEvent)>>,
    }

    impl IdbBackend {
        fn new(idb: crate::idb::IdbStore) -> Self {
            let channel =
                web_sys::BroadcastChannel::new("cadcraft.ws").map_err(|error| log::warn!("workspace: BroadcastChannel unavailable: {error:?}")).ok();
            Self { idb, channel, _listener: None }
        }
    }

    impl Drop for IdbBackend {
        fn drop(&mut self) {
            if let Some(channel) = &self.channel {
                channel.set_onmessage(None);
                channel.close();
            }
        }
    }

    impl Backend for IdbBackend {
        fn submit(&mut self, op: Op) -> Pending {
            match op {
                Op::Commit(c) => self.idb.commit_workspace(c),
                Op::Current(json) => self.idb.put_many("workspace", &[("current".into(), json.into_bytes())]),
                Op::Snapshot(revision, json) => self.idb.put_many("snapshots", &[(revision.to_string(), json.into_bytes())]),
                Op::KeepDamaged(id, bytes) => self.idb.keep_once("damaged", &id.to_string(), bytes, false),
                Op::PutMeta(key, value) => self.idb.keep_once("meta", &key, value.into_bytes(), true),
                Op::Evict(revisions) => self.idb.evict_snapshots(&revisions),
            }
        }

        fn broadcast(&mut self, revision: u64, json: &str) {
            let Some(channel) = &self.channel else { return };
            // Decimal text avoids losing u64 revision precision in JavaScript numbers.
            let message = js_sys::Object::new();
            let result = js_sys::Reflect::set(&message, &JsValue::from_str("revision"), &JsValue::from_str(&revision.to_string()))
                .and_then(|_| js_sys::Reflect::set(&message, &JsValue::from_str("json"), &JsValue::from_str(json)))
                .and_then(|_| channel.post_message(&message));
            if let Err(error) = result {
                log::warn!("workspace: could not broadcast commit: {error:?}");
            }
        }
    }

    impl IdbProfileStore<IdbBackend> {
        pub fn repaint_on_external_change(&mut self, ctx: &egui::Context) {
            let Some(channel) = &self.backend.channel else { return };
            let incoming = Rc::clone(&self.incoming);
            let ctx = ctx.clone();
            let listener = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                let message = event.data();
                let revision = js_sys::Reflect::get(&message, &JsValue::from_str("revision"))
                    .ok()
                    .and_then(|value| value.as_string())
                    .and_then(|value| value.parse::<u64>().ok());
                let json = js_sys::Reflect::get(&message, &JsValue::from_str("json")).ok().and_then(|value| value.as_string());
                if let (Some(revision), Some(json)) = (revision, json)
                    && json.len() <= MAX_SET_BYTES
                    && stored_revision(&json) == Some(revision)
                    && parse_set(&json).is_ok()
                    && let Ok(mut slot) = incoming.try_borrow_mut()
                    && slot.as_ref().is_none_or(|(old, _)| revision > *old)
                {
                    *slot = Some((revision, json));
                    ctx.request_repaint();
                }
            });
            channel.set_onmessage(Some(listener.as_ref().unchecked_ref()));
            self.backend._listener = Some(listener);
        }
    }

    pub struct Opened {
        pub profile: IdbProfileStore<IdbBackend>,
        pub autosave: crate::drawing_autosave::IdbAutosaveSink,
        pub leftovers: Vec<cadcraft_engine::autosave::Recoverable>,
        pub instance: String,
    }

    pub async fn open() -> Result<Opened, IdbError> {
        let (idb, mut mirror, index, beats) = crate::idb::with_timeout(async {
            let idb = crate::idb::IdbStore::open().await?;
            let mut mirror = Mirror::default();
            for name in ["workspace", "snapshots", "damaged", "meta"] {
                let records = idb.preload(name, MAX_PRELOAD_RECORDS).await?;
                if name == "workspace"
                    && records.iter().any(|(key, raw)| key == "current" && (raw.len() > MAX_SET_BYTES || std::str::from_utf8(raw).is_err()))
                {
                    return Err(IdbError::Unavailable("stored workspace exceeds its limits or is not UTF-8".into()));
                }
                mirror.preload(name, records);
            }
            let index = idb.preload("autosave_index", MAX_PRELOAD_RECORDS).await?;
            let beats = idb.preload("instances", MAX_PRELOAD_RECORDS).await?;
            Ok((idb, mirror, index, beats))
        })
        .await?;
        if mirror.needs_migration()
            && let Some(mut storage) = crate::profile_store::browser_storage()
            && let Some(mut imported) = migrate_from_local_storage(&storage)
        {
            for (key, value) in &mirror.meta {
                imported.meta.insert(key.clone(), value.clone());
            }
            let pending = idb.import_workspace(imported.clone());
            // Keep the source intact until the whole migration transaction is durable.
            match crate::idb::wait_pending(&pending).await {
                Ok(()) => {
                    cleanup_local_storage(&mut storage, &imported);
                    mirror = imported;
                }
                Err(IdbError::Conflict { .. }) => {
                    mirror = Mirror::default();
                    crate::idb::with_timeout(async {
                        for name in ["workspace", "snapshots", "damaged", "meta"] {
                            mirror.preload(name, idb.preload(name, MAX_PRELOAD_RECORDS).await?);
                        }
                        Ok(())
                    })
                    .await?;
                }
                Err(error) => return Err(error),
            }
        }
        let persisted = crate::idb::request_persistence().await;
        let now = js_sys::Date::now();
        let instance = crate::drawing_autosave::web_instance_id((js_sys::Math::random() * f64::from(u32::MAX)) as u32, now);
        let leftovers = crate::drawing_autosave::leftovers(&index, &beats, &instance, now);
        let autosave = crate::drawing_autosave::IdbAutosaveSink::new(idb.clone(), instance.clone(), js_sys::Date::now);
        let profile = IdbProfileStore::new(mirror, IdbBackend::new(idb), js_sys::Date::now, persisted);
        Ok(Opened { profile, autosave, leftovers, instance })
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{IdbBackend, open};

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use cadcraft_ui_egui::wsprofile::canonical::{ChecksumCheck, verify_checksum};
    use cadcraft_ui_egui::wsprofile::startup::fresh_user;
    use cadcraft_ui_egui::wsprofile::store::ProfileSet;

    #[derive(Default)]
    struct FakeBackend {
        ops: Vec<(Op, Pending)>,
        broadcasts: Vec<(u64, String)>,
    }

    impl Backend for FakeBackend {
        fn submit(&mut self, op: Op) -> Pending {
            let pending = Pending::default();
            self.ops.push((op, pending.clone()));
            pending
        }

        fn broadcast(&mut self, revision: u64, json: &str) {
            self.broadcasts.push((revision, json.into()));
        }
    }

    fn json(revision: u64) -> String {
        let mut user = fresh_user("SIMPLE", &[]);
        user.revision = revision;
        user.created_at = "1970-01-01T00:00:00Z".into();
        user.updated_at = user.created_at.clone();
        set_text(&ProfileSet { user: Some(user), ..ProfileSet::default() }, &[]).expect("test set")
    }

    fn store(mirror: Mirror) -> IdbProfileStore<FakeBackend> {
        IdbProfileStore::new(mirror, FakeBackend::default(), || 1_800_000_000_000.0, true)
    }

    fn commit(store: &mut IdbProfileStore<FakeBackend>, expected: u64, revision: u64, prune: Vec<u64>) {
        store.write_commit(Commit { expected, revision, json: &json(revision), prune }).expect("queue commit");
    }

    fn finish(store: &mut IdbProfileStore<FakeBackend>, index: usize, result: Result<(), IdbError>) {
        store.backend.ops.get(index).expect("queued operation").1.complete(result);
    }

    #[test]
    fn mirror_reads_what_was_preloaded() {
        let current = json(4);
        let snapshot = json(3);
        let mut mirror = Mirror::default();
        mirror.preload("workspace", [("current".into(), current.clone().into_bytes())]);
        mirror.preload("snapshots", [("3".into(), snapshot.clone().into_bytes())]);
        let store = store(mirror);
        assert_eq!(store.read_current(), Some(current));
        assert_eq!(store.snapshots(), vec![3]);
        assert_eq!(store.read_snapshot(3), Some(snapshot));
        assert_eq!(store.persisted_storage(), Some(true));
    }

    #[test]
    fn write_commit_updates_mirror_and_queues_one_op() {
        let mut store = store(Mirror::default());
        commit(&mut store, 0, 4, vec![1, 2]);
        assert_eq!(store.backend.ops.len(), 1);
        let Op::Commit(c) = &store.backend.ops.first().expect("commit").0 else { panic!("commit operation") };
        assert_eq!((c.expected, c.revision, c.prune.clone()), (0, 4, vec![1, 2]));
        assert_eq!(store.read_current(), Some(c.json.clone()));
        assert_eq!(store.read_snapshot(4), Some(c.json.clone()));
        assert_eq!(store.durability(), Durability::Pending);
        assert!(store.backend.broadcasts.is_empty());
        finish(&mut store, 0, Ok(()));
        assert_eq!(store.durability(), Durability::Committed);
        assert_eq!(store.backend.broadcasts.len(), 1);
        assert_eq!(store.durability(), Durability::Committed);
        assert_eq!(store.backend.broadcasts.len(), 1);
    }

    #[test]
    fn committed_mirror_checksum_survives_clock_stamping() {
        let mut store = store(Mirror::default());
        commit(&mut store, 0, 1, vec![]);
        finish(&mut store, 0, Ok(()));
        assert_eq!(store.durability(), Durability::Committed);
        let text = store.read_current().expect("committed text");
        assert!(!text.contains("1970-01-01T00:00:00"));
        let value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(verify_checksum(&value), ChecksumCheck::Valid);
        assert!(parse_set(&text).is_ok());
        assert_eq!(store.read_snapshot(1), Some(text));
    }

    #[test]
    fn quota_error_evicts_only_old_snapshots() {
        let current = json(3);
        let mut store = store(Mirror {
            current: Some(current.clone()),
            snapshots: [(0, "damaged".into()), (1, json(1)), (2, json(2)), (3, current.clone())].into(),
            damaged: [(1, b"evidence".to_vec())].into(),
            meta: [(BACKUP.into(), "original".into())].into(),
        });
        commit(&mut store, 3, 4, vec![]);
        finish(&mut store, 0, Err(IdbError::Quota));
        assert_eq!(store.durability(), Durability::Pending);
        assert_eq!(store.backend.ops.get(1).expect("eviction").0, Op::Evict(vec![1, 2]));
        finish(&mut store, 1, Ok(()));
        assert_eq!(store.durability(), Durability::Pending);
        assert!(matches!(store.backend.ops.get(2).expect("retry").0, Op::Commit(_)));
        finish(&mut store, 2, Err(IdbError::Quota));
        assert_eq!(store.durability(), Durability::Failed("Browser storage is full".into()));
        assert_eq!(store.read_current(), Some(current.clone()));
        assert_eq!(store.read_snapshot(3), Some(current));
        assert_eq!(store.snapshots(), vec![0, 3]);
        assert_eq!(store.mirror.damaged.get(&1), Some(&b"evidence".to_vec()));
        assert_eq!(store.read_backup("1.0.0"), Some("original".into()));
        assert!(store.backend.broadcasts.is_empty());
        assert_eq!(store.backend.ops.len(), 3);
    }

    #[test]
    fn quota_retry_can_commit_without_deleting_the_last_snapshot() {
        let mut store = store(Mirror { current: Some(json(1)), snapshots: [(1, json(1))].into(), ..Mirror::default() });
        commit(&mut store, 1, 2, vec![]);
        finish(&mut store, 0, Err(IdbError::Quota));
        assert_eq!(store.durability(), Durability::Pending);
        assert!(matches!(store.backend.ops.get(1).expect("retry").0, Op::Commit(_)));
        finish(&mut store, 1, Ok(()));
        assert_eq!(store.durability(), Durability::Committed);
        assert_eq!(store.snapshots(), vec![1, 2]);
    }

    #[test]
    fn conflict_from_transaction_is_reported() {
        let original = Mirror { current: Some(json(3)), ..Mirror::default() };
        let mut store = store(original.clone());
        commit(&mut store, 3, 4, vec![]);
        finish(&mut store, 0, Err(IdbError::Conflict { stored: 7 }));
        assert_eq!(store.durability(), Durability::Conflict { stored: 7 });
        assert_eq!(store.mirror, original);
        assert!(store.backend.broadcasts.is_empty());
    }

    #[test]
    fn broadcast_from_another_tab_updates_mirror_and_flags_change() {
        let mut store = store(Mirror { current: Some(json(3)), ..Mirror::default() });
        store.on_broadcast(9, json(9));
        assert_eq!(store.read_current(), Some(json(9)));
        assert!(store.take_external_change());
        assert!(!store.take_external_change());
        store.on_broadcast(8, json(8));
        assert_eq!(store.read_current(), Some(json(9)));
        assert!(!store.take_external_change());
        store.on_broadcast(10, "broken".into());
        assert!(!store.take_external_change());
        assert_eq!(store.write_commit(Commit { expected: 8, revision: 10, json: &json(10), prune: vec![] }), Err(TAB_CONFLICT_MESSAGE.into()));
    }

    #[derive(Default)]
    struct FakeStorage(BTreeMap<String, String>);

    impl KeyValue for FakeStorage {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
            self.0.insert(key.into(), value.into());
            Ok(())
        }
        fn remove(&mut self, key: &str) {
            self.0.remove(key);
        }
    }

    #[test]
    fn local_storage_set_is_copied_once() {
        let mut kv = FakeStorage::default();
        kv.set(CURRENT_KEY, &json(3)).expect("current");
        kv.set(INDEX_KEY, "[1,2]").expect("index");
        kv.set(&snapshot_key(1), &json(1)).expect("snapshot");
        kv.set(&snapshot_key(2), &json(2)).expect("snapshot");
        kv.set(DAMAGED_INDEX_KEY, "[4]").expect("damaged index");
        kv.set(&damaged_key(4), "broken").expect("damaged");
        kv.set(LOCAL_BACKUP, "original v1").expect("backup");
        let mut mirror = Mirror::default();
        assert!(mirror.needs_migration());
        mirror = migrate_from_local_storage(&kv).expect("migration");
        assert_eq!(mirror.current, Some(json(3)));
        assert_eq!(mirror.snapshots.len(), 2);
        assert_eq!(mirror.damaged.get(&4), Some(&b"broken".to_vec()));
        assert_eq!(mirror.meta.get(BACKUP), Some(&"original v1".into()));
        cleanup_local_storage(&mut kv, &mirror);
        assert_eq!(kv.get(CURRENT_KEY), Some(json(3)));
        assert_eq!(kv.get(LOCAL_BACKUP), Some("original v1".into()));
        assert!(kv.get(&snapshot_key(1)).is_none());
        assert!(kv.get(&snapshot_key(2)).is_none());
        assert!(kv.get(&damaged_key(4)).is_none());
        assert!(kv.get(INDEX_KEY).is_none());
        assert!(kv.get(DAMAGED_INDEX_KEY).is_none());
        mirror.current = None;
        mirror.snapshots.clear();
        mirror.damaged.clear();
        assert!(!mirror.needs_migration());
    }

    #[test]
    fn preload_is_capped() {
        let records = (0..=MAX_PRELOAD_RECORDS).map(|revision| (revision.to_string(), b"{}".to_vec()));
        let mut mirror = Mirror::default();
        mirror.preload("snapshots", records);
        assert_eq!(mirror.snapshots.len(), MAX_PRELOAD_RECORDS);
        assert!(!mirror.snapshots.contains_key(&(MAX_PRELOAD_RECORDS as u64)));
    }

    #[test]
    fn failed_backup_prevents_the_dependent_commit() {
        let mut store = store(Mirror::default());
        store.keep_backup("1.0.0", "original").expect("queue backup");
        commit(&mut store, 0, 1, vec![]);
        assert_eq!(store.backend.ops.len(), 1);
        assert_eq!(store.durability(), Durability::Pending);
        finish(&mut store, 0, Err(IdbError::Unavailable("blocked".into())));
        assert_eq!(store.durability(), Durability::Failed("blocked".into()));
        assert!(store.read_current().is_none());
        assert!(store.read_backup("1.0.0").is_none());
        assert_eq!(store.backend.ops.len(), 1);
    }

    #[test]
    fn backup_and_damaged_copies_are_written_before_current() {
        let mut store = store(Mirror::default());
        store.keep_backup("1.0.0", "original").expect("backup");
        store.keep_backup("1.0.0", "replacement").expect("idempotent");
        store.keep_damaged(b"broken").expect("damage");
        store.push_snapshot(0, "evidence").expect("snapshot");
        store.write_current("original").expect("rollback");
        assert_eq!(store.backend.ops.len(), 1);
        for index in 0..4 {
            finish(&mut store, index, Ok(()));
            let expected = if index == 3 { Durability::Committed } else { Durability::Pending };
            assert_eq!(store.durability(), expected);
        }
        assert_eq!(store.read_backup("1.0.0"), Some("original".into()));
        assert_eq!(store.read_current(), Some("original".into()));
        assert!(matches!(store.backend.ops.get(3).expect("current").0, Op::Current(_)));
    }

    #[test]
    fn aborted_commit_restores_the_previous_mirror() {
        let original = Mirror { current: Some(json(1)), snapshots: [(1, json(1))].into(), ..Mirror::default() };
        let mut store = store(original.clone());
        commit(&mut store, 1, 2, vec![1]);
        finish(&mut store, 0, Err(IdbError::Aborted("interrupted".into())));
        assert_eq!(store.durability(), Durability::Failed("interrupted".into()));
        assert_eq!(store.mirror, original);
    }

    #[test]
    fn conflict_exposes_the_stored_set_without_a_broadcast() {
        let mut store = store(Mirror { current: Some(json(3)), ..Mirror::default() });
        commit(&mut store, 3, 4, vec![]);
        store.backend.ops.first().expect("commit").1.observe_current(json(7));
        finish(&mut store, 0, Err(IdbError::Conflict { stored: 7 }));
        assert_eq!(store.durability(), Durability::Conflict { stored: 7 });
        assert_eq!(store.read_current(), Some(json(7)));
    }

    #[test]
    fn failed_eviction_preserves_the_previous_history() {
        let original = Mirror { current: Some(json(3)), snapshots: [(1, json(1)), (2, json(2)), (3, json(3))].into(), ..Mirror::default() };
        let mut store = store(original.clone());
        commit(&mut store, 3, 4, vec![]);
        finish(&mut store, 0, Err(IdbError::Quota));
        assert_eq!(store.durability(), Durability::Pending);
        finish(&mut store, 1, Err(IdbError::Aborted("eviction failed".into())));
        assert_eq!(store.durability(), Durability::Failed("eviction failed".into()));
        assert_eq!(store.mirror, original);
    }

    #[test]
    fn failed_backup_is_retried_before_a_later_commit() {
        let mut store = store(Mirror::default());
        store.keep_backup("1.0.0", "original").expect("backup");
        commit(&mut store, 0, 1, vec![]);
        finish(&mut store, 0, Err(IdbError::Quota));
        assert_eq!(store.durability(), Durability::Failed("Browser storage is full".into()));
        commit(&mut store, 0, 2, vec![]);
        assert_eq!(store.backend.ops.get(1).expect("backup retry").0, Op::PutMeta(BACKUP.into(), "original".into()));
        finish(&mut store, 1, Ok(()));
        assert_eq!(store.durability(), Durability::Pending);
        finish(&mut store, 2, Ok(()));
        assert_eq!(store.durability(), Durability::Committed);
        assert_eq!(store.read_backup("1.0.0"), Some("original".into()));
    }

    #[test]
    fn broadcast_waits_for_the_pending_commit_outcome() {
        let mut store = store(Mirror { current: Some(json(3)), ..Mirror::default() });
        commit(&mut store, 3, 4, vec![]);
        store.on_broadcast(9, json(9));
        assert_eq!(store.read_current().as_deref().and_then(stored_revision), Some(4));
        assert!(!store.take_external_change());
        finish(&mut store, 0, Err(IdbError::Conflict { stored: 9 }));
        assert_eq!(store.durability(), Durability::Conflict { stored: 9 });
        assert_eq!(store.read_current(), Some(json(9)));
        assert!(store.take_external_change());
    }

    #[test]
    fn legacy_migration_keeps_the_original_and_signs_v3() {
        let value: serde_json::Value = serde_json::from_str(&json(3)).expect("wire set");
        let mut legacy = cadcraft_ui_egui::wsprofile::wire::from_wire(&value);
        legacy.as_object_mut().expect("set object").remove("checksum");
        legacy["schema_version"] = serde_json::json!("1.0.0");
        legacy["user"]["schema_version"] = serde_json::json!("1.0.0");
        let original = legacy.to_string();
        assert!(parse_set(&original).is_ok());
        let mut kv = FakeStorage::default();
        kv.set(CURRENT_KEY, &original).expect("legacy current");
        let mirror = migrate_from_local_storage(&kv).expect("migration");
        assert_eq!(mirror.meta.get(BACKUP), Some(&original));
        assert_eq!(kv.get(CURRENT_KEY), Some(original));
        let converted: serde_json::Value = serde_json::from_str(mirror.current.as_deref().expect("current")).expect("wire v3");
        assert_eq!(converted.get("schemaVersion"), Some(&serde_json::json!(3)));
        assert_eq!(verify_checksum(&converted), ChecksumCheck::Valid);
    }

    #[test]
    fn cleanup_preserves_concurrent_local_storage_edits() {
        let mut kv = FakeStorage::default();
        kv.set(CURRENT_KEY, &json(1)).expect("current");
        kv.set(INDEX_KEY, "[1]").expect("index");
        kv.set(&snapshot_key(1), &json(1)).expect("snapshot");
        let mirror = migrate_from_local_storage(&kv).expect("migration");
        kv.set(INDEX_KEY, "[1,2]").expect("concurrent index");
        kv.set(&snapshot_key(2), &json(2)).expect("concurrent snapshot");
        cleanup_local_storage(&mut kv, &mirror);
        assert_eq!(kv.get(INDEX_KEY), Some("[1,2]".into()));
        assert_eq!(kv.get(&snapshot_key(2)), Some(json(2)));
        assert_eq!(kv.get(CURRENT_KEY), Some(json(1)));
    }
}

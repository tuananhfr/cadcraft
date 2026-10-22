//! Drawing persistence shares the workspace connection but owns no workspace eviction policy.

use std::cell::RefCell;
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

use cadcraft_engine::autosave::{Recoverable, STALE_LOCK_SECS, is_instance_id, parse_file_name};
use cadcraft_ui_egui::autosave::{AutosaveSink, MAX_WEB_AUTOSAVE_BYTES};
use serde_json::{Value, json};

use crate::idb_profile_store::{MAX_PRELOAD_RECORDS, Pending};

#[cfg(target_arch = "wasm32")]
pub type IdbAutosaveSink = DrawingAutosave<crate::idb::IdbStore>;

const MAX_PENDING: usize = 64;
const MAX_METADATA_BYTES: usize = 4096;
type Fetched = (String, Result<Vec<u8>, String>);
pub type FetchSlot = Rc<RefCell<Option<Fetched>>>;

pub fn web_instance_id(random: u32, now_ms: f64) -> String {
    let stamp = milliseconds(now_ms).unwrap_or(0);
    format!("{random}-{stamp:x}")
}

fn milliseconds(value: f64) -> Option<u64> {
    if value.is_finite() && (0.0..u64::MAX as f64).contains(&value) { Some(value as u64) } else { None }
}

pub fn leftovers(index: &[(String, Vec<u8>)], beats: &[(String, Vec<u8>)], own: &str, now_ms: f64) -> Vec<Recoverable> {
    let Some(now) = milliseconds(now_ms) else { return Vec::new() };
    let live: BTreeMap<_, _> = beats
        .iter()
        .take(MAX_PRELOAD_RECORDS)
        .filter_map(|(name, bytes)| {
            if !is_instance_id(name) || bytes.len() > MAX_METADATA_BYTES {
                return None;
            }
            let beat = serde_json::from_slice::<Value>(bytes).ok()?.as_f64()?;
            let beat = milliseconds(beat)?;
            Some((name.as_str(), now.saturating_sub(beat) < STALE_LOCK_SECS * 1000))
        })
        .collect();
    let mut result = Vec::new();
    for (name, raw) in index.iter().take(MAX_PRELOAD_RECORDS) {
        if name.len() > MAX_METADATA_BYTES || raw.len() > MAX_METADATA_BYTES || parse_file_name(name).is_none() {
            continue;
        }
        let Ok(meta) = serde_json::from_slice::<Value>(raw) else { continue };
        let Some(instance) = meta.get("instance").and_then(Value::as_str).filter(|instance| is_instance_id(instance)) else { continue };
        if instance == own || live.get(instance).copied().unwrap_or(false) {
            continue;
        }
        let Some(title) = meta.get("title").and_then(Value::as_str).filter(|title| title.len() <= MAX_METADATA_BYTES) else { continue };
        let Some(modified) = meta.get("modified_ms").and_then(Value::as_f64).and_then(milliseconds) else { continue };
        let Some(bytes) = meta.get("bytes").and_then(Value::as_u64).filter(|bytes| *bytes <= MAX_WEB_AUTOSAVE_BYTES as u64) else { continue };
        result.push(Recoverable { path: name.clone(), drawing: title.into(), modified: modified / 1000, bytes });
    }
    result.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));
    result
}

pub trait Backend {
    fn write(&self, name: &str, bytes: &[u8], metadata: &[u8]) -> Pending;
    fn delete(&self, name: &str) -> Pending;
    fn beat(&self, instance: &str, bytes: Vec<u8>) -> Pending;
    fn fetch(&self, name: String, slot: FetchSlot, ctx: Option<egui::Context>);
}

struct Writing {
    name: String,
    at_ms: f64,
    pending: Pending,
}

pub struct DrawingAutosave<B: Backend> {
    store: B,
    instance: String,
    clock: fn() -> f64,
    ctx: Option<egui::Context>,
    writes: Vec<Writing>,
    durable: BTreeMap<String, f64>,
    failed: VecDeque<(String, String)>,
    fetched: FetchSlot,
    fetching: bool,
}

impl<B: Backend> DrawingAutosave<B> {
    pub fn new(store: B, instance: String, clock: fn() -> f64) -> Self {
        Self {
            store,
            instance,
            clock,
            ctx: None,
            writes: Vec::new(),
            durable: BTreeMap::new(),
            failed: VecDeque::new(),
            fetched: Rc::new(RefCell::new(None)),
            fetching: false,
        }
    }

    pub fn set_context(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
    }

    fn track(&mut self, name: &str, at_ms: f64, pending: Pending) {
        self.writes.push(Writing { name: name.into(), at_ms, pending });
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn reap(&mut self) {
        let mut pending = Vec::new();
        for write in self.writes.drain(..) {
            match write.pending.result() {
                None => pending.push(write),
                Some(Ok(())) => {
                    if write.at_ms >= 0.0 {
                        self.durable.insert(write.name, write.at_ms);
                    }
                }
                Some(Err(error)) => {
                    self.durable.remove(&write.name);
                    if self.failed.len() < MAX_PENDING {
                        self.failed.push_back((write.name, error.message()));
                    }
                }
            }
        }
        self.writes = pending;
        if !self.writes.is_empty()
            && let Some(ctx) = &self.ctx
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl<B: Backend> AutosaveSink for DrawingAutosave<B> {
    fn write(&mut self, name: &str, title: &str, bytes: Vec<u8>) -> Result<(), String> {
        self.reap();
        if parse_file_name(name).is_none() || !is_instance_id(&self.instance) || title.len() > 1024 {
            return Err(cadcraft_ui_egui::tl!("Could not autosave the drawing").into());
        }
        if bytes.len() > MAX_WEB_AUTOSAVE_BYTES {
            return Err(cadcraft_ui_egui::tl!("Drawing too large to autosave in the browser").into());
        }
        if self.writes.len() >= MAX_PENDING {
            return Err(cadcraft_ui_egui::tl!("Drawing autosave queue is full").into());
        }
        let now = (self.clock)();
        if milliseconds(now).is_none() {
            return Err(cadcraft_ui_egui::tl!("Could not autosave the drawing").into());
        }
        let latest = self.writes.iter().filter(|write| write.name == name).map(|write| write.at_ms).reduce(f64::max);
        let previous = latest.or_else(|| self.durable.get(name).copied()).unwrap_or(-1.0);
        let at_ms = now.max(previous + 1.0);
        let metadata = json!({"title": title, "instance": self.instance, "modified_ms": at_ms, "bytes": bytes.len()}).to_string();
        let pending = self.store.write(name, &bytes, metadata.as_bytes());
        self.track(name, at_ms, pending);
        Ok(())
    }

    fn delete(&mut self, name: &str) {
        self.reap();
        if parse_file_name(name).is_none() {
            return;
        }
        // Dropped observations cannot resurrect a deleted drawing's durable status.
        self.writes.retain(|write| write.name != name);
        self.durable.remove(name);
        if self.writes.len() >= MAX_PENDING {
            if self.failed.len() < MAX_PENDING {
                self.failed.push_back((name.into(), cadcraft_ui_egui::tl!("Drawing autosave queue is full").into()));
            }
            return;
        }
        let pending = self.store.delete(name);
        self.track(name, -1.0, pending);
    }

    fn beat(&mut self, instance: &str, _now_ms: f64) {
        self.reap();
        if instance != self.instance || !is_instance_id(instance) || self.writes.len() >= MAX_PENDING {
            return;
        }
        let now = (self.clock)();
        if milliseconds(now).is_none() {
            return;
        }
        let pending = self.store.beat(instance, json!(now).to_string().into_bytes());
        self.track(instance, -1.0, pending);
    }

    fn fetch(&mut self, name: &str) {
        if self.fetching || parse_file_name(name).is_none() {
            return;
        }
        self.fetching = true;
        self.store.fetch(name.into(), Rc::clone(&self.fetched), self.ctx.clone());
    }

    fn take_fetched(&mut self) -> Option<Fetched> {
        let fetched = self.fetched.try_borrow_mut().ok()?.take();
        if fetched.is_some() {
            self.fetching = false;
        }
        fetched
    }

    fn last_written(&self, name: &str) -> Option<f64> {
        if let Some(latest) = self.writes.iter().rev().find(|write| write.name == name) {
            return (latest.at_ms >= 0.0 && matches!(latest.pending.result(), Some(Ok(())))).then_some(latest.at_ms);
        }
        self.durable.get(name).copied()
    }

    fn take_failed(&mut self) -> Option<(String, String)> {
        self.reap();
        self.failed.pop_front().map(|(name, error)| {
            let error = if error == crate::idb_profile_store::QUOTA_MESSAGE {
                cadcraft_ui_egui::tl!("Browser storage is full").into()
            } else {
                cadcraft_ui_egui::tf!("Could not autosave the drawing: {error}", error = error)
            };
            (name, error)
        })
    }
}

#[cfg(target_arch = "wasm32")]
impl Backend for crate::idb::IdbStore {
    fn write(&self, name: &str, bytes: &[u8], metadata: &[u8]) -> Pending {
        self.put_drawing(name, bytes, metadata)
    }

    fn delete(&self, name: &str) -> Pending {
        self.delete_drawing(name)
    }

    fn beat(&self, instance: &str, bytes: Vec<u8>) -> Pending {
        self.put_many("instances", &[(instance.into(), bytes)])
    }

    fn fetch(&self, name: String, slot: FetchSlot, ctx: Option<egui::Context>) {
        let store = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::idb::with_timeout(store.read_bounded("autosave", &name, MAX_WEB_AUTOSAVE_BYTES)).await.map_err(|e| e.message());
            if let Ok(mut slot) = slot.try_borrow_mut() {
                *slot = Some((name, result));
            }
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idb_profile_store::IdbError;

    #[derive(Default)]
    struct Fake {
        records: RefCell<BTreeMap<(String, String), Vec<u8>>>,
        writes: RefCell<Vec<(String, Pending)>>,
    }

    impl Backend for Rc<Fake> {
        fn write(&self, name: &str, bytes: &[u8], metadata: &[u8]) -> Pending {
            assert!(bytes.len() <= MAX_WEB_AUTOSAVE_BYTES);
            assert!(serde_json::from_slice::<Value>(metadata).is_ok());
            let pending = Pending::default();
            self.writes.borrow_mut().push((name.into(), pending.clone()));
            pending
        }
        fn delete(&self, name: &str) -> Pending {
            let mut records = self.records.borrow_mut();
            records.remove(&("autosave".into(), name.into()));
            records.remove(&("autosave_index".into(), name.into()));
            let pending = Pending::default();
            pending.complete(Ok(()));
            pending
        }
        fn beat(&self, instance: &str, bytes: Vec<u8>) -> Pending {
            self.records.borrow_mut().insert(("instances".into(), instance.into()), bytes);
            let pending = Pending::default();
            pending.complete(Ok(()));
            pending
        }
        fn fetch(&self, name: String, slot: FetchSlot, _ctx: Option<egui::Context>) {
            let result = self.records.borrow().get(&("autosave".into(), name.clone())).cloned().ok_or_else(|| "missing".into());
            *slot.borrow_mut() = Some((name, result));
        }
    }

    fn index(name: &str, instance: &str) -> (String, Vec<u8>) {
        (name.into(), json!({"title": "Plan", "instance": instance, "modified_ms": 1_000_000, "bytes": 42}).to_string().into_bytes())
    }

    #[test]
    fn web_instance_id_is_valid() {
        for random in [0, 1, u32::MAX] {
            for now in [0.0, 1_800_000_000_000.0, f64::NAN, f64::INFINITY, -1.0, u64::MAX as f64] {
                assert!(is_instance_id(&web_instance_id(random, now)));
            }
        }
        assert_eq!(web_instance_id(42, 255.0), "42-ff");
    }

    #[test]
    fn leftovers_skip_live_and_own_instances() {
        let index = vec![
            index("Own_1-a_1.sv$", "1-a"),
            index("Live_2-a_2.sv$", "2-a"),
            index("Dead_3-a_3.sv$", "3-a"),
            index("Missing_4-a_4.sv$", "4-a"),
            index("Future_5-a_5.sv$", "5-a"),
            index("Adopted_8-a_8.sv$", "2-a"),
        ];
        let beats = vec![("2-a".into(), b"999999".to_vec()), ("3-a".into(), b"700000".to_vec()), ("5-a".into(), b"1100000".to_vec())];
        let items = leftovers(&index, &beats, "1-a", 1_000_000.0);
        assert_eq!(items.len(), 2);
        assert_eq!(items.first().unwrap().path, "Dead_3-a_3.sv$");
        assert_eq!(items.last().unwrap().path, "Missing_4-a_4.sv$");
        assert_eq!(items.first().unwrap().modified, 1000);
        assert_eq!(items.first().unwrap().bytes, 42);
    }

    #[test]
    fn leftovers_reject_hostile_metadata_and_cap_the_preload() {
        let mut index = vec![
            ("Wrong_3-a_1.sv$".into(), b"bad".to_vec()),
            ("Big_3-a_1.sv$".into(), vec![b' '; MAX_METADATA_BYTES + 1]),
            ("Bad_3-a_1.sv$".into(), json!({"title":"x", "instance":"3-a", "modified_ms":-1, "bytes":1}).to_string().into_bytes()),
        ];
        assert!(leftovers(&index, &[], "1-a", 1_000_000.0).is_empty());
        assert!(leftovers(&[], &[], "1-a", f64::NAN).is_empty());
        index.clear();
        index.extend((0..=MAX_PRELOAD_RECORDS).map(|uid| self::index(&format!("Plan_3-a_{uid}.sv$"), "3-a")));
        assert_eq!(leftovers(&index, &[], "1-a", 1_000_000.0).len(), MAX_PRELOAD_RECORDS);
    }

    #[test]
    fn drawing_autosave_never_evicts_workspace() {
        let store = Rc::new(Fake::default());
        for name in ["workspace", "snapshots", "damaged", "meta"] {
            store.records.borrow_mut().insert((name.into(), "keep".into()), b"evidence".to_vec());
        }
        let before = store.records.borrow().clone();
        let mut sink = DrawingAutosave::new(store.clone(), "1-a".into(), || 1_000_000.0);
        let name = "Plan_1-a_1.sv$";
        sink.write(name, "Plan", b"drawing".to_vec()).unwrap();
        assert_eq!(sink.last_written(name), None, "queue acceptance is not durability");
        store.writes.borrow().first().unwrap().1.complete(Err(IdbError::Quota));
        assert_eq!(sink.take_failed(), Some((name.into(), "Browser storage is full".into())));
        assert_eq!(sink.last_written(name), None);
        assert_eq!(*store.records.borrow(), before);
        assert_eq!(store.writes.borrow().len(), 1, "quota errors do not trigger workspace eviction or automatic quota retries");
    }

    #[test]
    fn drawing_sink_marks_only_completed_transactions_durable() {
        let store = Rc::new(Fake::default());
        let mut sink = DrawingAutosave::new(store.clone(), "1-a".into(), || 1_000_000.0);
        let name = "Plan_1-a_1.sv$";
        sink.write(name, "Plan", vec![1]).unwrap();
        let pending = store.writes.borrow().first().unwrap().1.clone();
        assert_eq!(sink.last_written(name), None);
        pending.complete(Ok(()));
        assert_eq!(sink.last_written(name), Some(1_000_000.0));
        sink.write(name, "Plan", vec![2]).unwrap();
        assert_eq!(sink.last_written(name), None);
        store.writes.borrow().last().unwrap().1.complete(Ok(()));
        assert_eq!(sink.last_written(name), Some(1_000_001.0));
        sink.delete(name);
        assert_eq!(sink.last_written(name), None);
    }
}

//! The browser profile store: the workspace profile set and its snapshots in `localStorage`.
//! `localStorage` is a few MB for the whole site (fonts share it), so a full or blocked storage is
//! an `Err` the app shows, never a panic.

use std::cell::Cell;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use cadcraft_ui_egui::wsprofile::rfc3339;
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES, ProfileStore, expired_snapshots};
use serde_json::Value;

const BACKUP_KEY: &str = "cadcraft.ws.backup:1.0.0";

pub const CURRENT_KEY: &str = "cadcraft.ws.current";
pub const SNAPSHOT_PREFIX: &str = "cadcraft.ws.snap:";
/// The revisions kept, as a JSON list; `localStorage` cannot list keys by prefix cheaply.
pub const INDEX_KEY: &str = "cadcraft.ws.snaps";
/// Damaged copies of a set startup could not read; listed apart from the snapshots so pruning one never touches the other.
pub const DAMAGED_PREFIX: &str = "cadcraft.ws.damaged:";
pub const DAMAGED_INDEX_KEY: &str = "cadcraft.ws.damaged";
/// They are evidence, not history, but the storage is small: the oldest go first.
const MAX_DAMAGED_COPIES: usize = 3;

/// The start of every time the core stamps while it has no clock (the web build has none).
const EPOCH_PREFIX: &str = "1970-01-01T00:00:00";

pub fn is_profile_key(key: Option<&str>) -> bool {
    key.is_none_or(|key| key == CURRENT_KEY)
}

pub fn snapshot_key(revision: u64) -> String {
    format!("{SNAPSHOT_PREFIX}{revision}")
}

pub fn damaged_key(id: u64) -> String {
    format!("{DAMAGED_PREFIX}{id}")
}

/// What the store needs from `localStorage`; a fake stands in for it in tests.
pub trait KeyValue {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: &str) -> Result<(), String>;
    fn remove(&mut self, key: &str);
}

fn numbers(text: &str) -> Vec<u64> {
    let list = serde_json::from_str::<Value>(text).ok().and_then(|value| value.as_array().cloned()).unwrap_or_default();
    list.iter().filter_map(Value::as_u64).collect()
}

/// Reads a saved index; anything unreadable is dropped, and a hostile one is cut to what the store keeps.
pub fn parse_index(text: &str) -> Vec<u64> {
    let all = numbers(text);
    let damaged_copy = all.contains(&CORRUPT_SNAPSHOT);
    let mut revisions: Vec<u64> = all.into_iter().filter(|revision| *revision != CORRUPT_SNAPSHOT).collect();
    revisions.sort_unstable();
    revisions.dedup();
    let excess = revisions.len().saturating_sub(KEEP_SNAPSHOTS);
    revisions.drain(..excess);
    if damaged_copy {
        revisions.insert(0, CORRUPT_SNAPSHOT);
    }
    revisions
}

pub fn index_json(revisions: &[u64]) -> String {
    Value::Array(revisions.iter().map(|revision| Value::from(*revision)).collect()).to_string()
}

fn parse_damaged_index(text: &str) -> Vec<u64> {
    let mut ids = numbers(text);
    ids.sort_unstable();
    ids.dedup();
    let excess = ids.len().saturating_sub(MAX_DAMAGED_COPIES);
    ids.drain(..excess);
    ids
}

/// The profile objects of a stored set: the working profile and the saved ones.
fn profiles_mut(set: &mut Value) -> Vec<&mut Value> {
    let mut found = Vec::new();
    if let Value::Object(fields) = set {
        for (key, value) in fields.iter_mut() {
            match (key.as_str(), value) {
                ("user", user) => found.push(user),
                ("profiles", Value::Array(list)) => found.extend(list.iter_mut()),
                _ => {}
            }
        }
    }
    found
}

/// Notes the creation time of each profile in a stored set, so a later placeholder gets the same one.
pub fn remember_created(json: &str, created: &mut BTreeMap<String, String>) {
    let Ok(mut set) = serde_json::from_str::<Value>(json) else { return };
    for profile in profiles_mut(&mut set) {
        let id = profile.get("profileId").or_else(|| profile.get("profile_id")).and_then(Value::as_str);
        let time =
            profile.get("createdAt").or_else(|| profile.get("created_at")).and_then(Value::as_str).filter(|time| !time.starts_with(EPOCH_PREFIX));
        if let (Some(id), Some(time)) = (id, time) {
            created.entry(id.to_string()).or_insert_with(|| time.to_string());
        }
    }
}

/// Replaces the 1970 times the core writes on the web (no clock there). `updated_at` becomes `now`;
/// `created_at` becomes the first time this profile was stamped, so it stays put across saves.
pub fn stamp_clock(json: &str, now: &str, created: &mut BTreeMap<String, String>) -> String {
    if !json.contains(EPOCH_PREFIX) {
        return json.to_string();
    }
    let Ok(mut set) = serde_json::from_str::<Value>(json) else { return json.to_string() };
    let mut changed = false;
    for profile in profiles_mut(&mut set) {
        let wire = profile.get("schemaVersion").is_some();
        let id = profile.get(if wire { "profileId" } else { "profile_id" }).and_then(Value::as_str).unwrap_or_default().to_string();
        for field in if wire { ["createdAt", "updatedAt"] } else { ["created_at", "updated_at"] } {
            let Some(slot) = profile.get_mut(field) else { continue };
            if !slot.as_str().is_some_and(|time| time.starts_with(EPOCH_PREFIX)) {
                continue;
            }
            let time = if matches!(field, "created_at" | "createdAt") {
                created.entry(id.clone()).or_insert_with(|| now.to_string()).clone()
            } else {
                now.to_string()
            };
            *slot = Value::String(time);
            changed = true;
        }
    }
    if !changed {
        return json.to_string();
    }
    if set.get("checksum").is_some() {
        set = cadcraft_ui_egui::wsprofile::canonical::with_checksum(set);
    }
    serde_json::to_string(&set).unwrap_or_else(|_| json.to_string())
}

fn within(json: &str) -> Result<(), String> {
    if json.len() > MAX_SET_BYTES { Err(format!("the workspace is larger than {MAX_SET_BYTES} bytes")) } else { Ok(()) }
}

pub struct LocalStorageStore<K: KeyValue> {
    kv: K,
    now_ms: fn() -> f64,
    created: BTreeMap<String, String>,
    external: Rc<Cell<bool>>,
    #[cfg(target_arch = "wasm32")]
    repaint: Rc<RefCell<Option<egui::Context>>>,
}

impl<K: KeyValue> LocalStorageStore<K> {
    pub fn new(kv: K, now_ms: fn() -> f64) -> Self {
        let mut created = BTreeMap::new();
        if let Some(stored) = kv.get(CURRENT_KEY) {
            remember_created(&stored, &mut created);
        }
        Self {
            kv,
            now_ms,
            created,
            external: Rc::new(Cell::new(false)),
            #[cfg(target_arch = "wasm32")]
            repaint: Rc::new(RefCell::new(None)),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn repaint_on_external_change(&mut self, ctx: &egui::Context) {
        if let Ok(mut repaint) = self.repaint.try_borrow_mut() {
            *repaint = Some(ctx.clone());
        }
    }

    #[cfg(test)]
    fn storage(&self) -> &K {
        &self.kv
    }

    fn read_index(&self) -> Vec<u64> {
        self.kv.get(INDEX_KEY).map(|text| parse_index(&text)).unwrap_or_default()
    }

    fn stamped(&mut self, json: &str) -> String {
        stamp_clock(json, &rfc3339((self.now_ms)()), &mut self.created)
    }

    /// Drops the oldest snapshot (never the damaged-copy marker or `keep`) from `index` and storage.
    fn evict_oldest(&mut self, index: &mut Vec<u64>, keep: Option<u64>) -> bool {
        let Some(oldest) = index.iter().copied().find(|revision| *revision != CORRUPT_SNAPSHOT && Some(*revision) != keep) else { return false };
        log::warn!("workspace: browser storage is full; dropping snapshot {oldest}");
        index.retain(|revision| *revision != oldest);
        self.kv.remove(&snapshot_key(oldest));
        true
    }

    /// Stores `value`; when the storage is full, the oldest snapshots in `index` make room (the
    /// current set matters more than its history), one at a time, until it fits or none is left.
    fn set_evicting(&mut self, key: &str, value: &str, index: &mut Vec<u64>, keep: Option<u64>) -> Result<(), String> {
        loop {
            match self.kv.set(key, value) {
                Ok(()) => return Ok(()),
                Err(error) if !self.evict_oldest(index, keep) => return Err(error),
                Err(_) => {}
            }
        }
    }

    /// Writes the index, giving up the oldest snapshots while it does not fit.
    fn save_index(&mut self, index: &mut Vec<u64>, keep: Option<u64>) -> Result<(), String> {
        loop {
            match self.kv.set(INDEX_KEY, &index_json(index)) {
                Ok(()) => return Ok(()),
                Err(error) if !self.evict_oldest(index, keep) => return Err(error),
                Err(_) => {}
            }
        }
    }

    fn damaged_ids(&self) -> Vec<u64> {
        self.kv.get(DAMAGED_INDEX_KEY).map(|text| parse_damaged_index(&text)).unwrap_or_default()
    }

    /// Frees the room of the oldest damaged copy; false when there is none.
    fn drop_oldest_damaged(&mut self, ids: &mut Vec<u64>) -> bool {
        if ids.is_empty() {
            return false;
        }
        let oldest = ids.remove(0);
        self.kv.remove(&damaged_key(oldest));
        true
    }
}

impl<K: KeyValue> ProfileStore for LocalStorageStore<K> {
    fn keep_backup(&mut self, version: &str, raw: &str) -> Result<(), String> {
        if version != "1.0.0" {
            return Err("unsupported backup version".into());
        }
        within(raw)?;
        if self.kv.get(BACKUP_KEY).is_none() {
            self.kv.set(BACKUP_KEY, raw)?;
        }
        Ok(())
    }

    fn read_backup(&self, version: &str) -> Option<String> {
        (version == "1.0.0").then(|| self.kv.get(BACKUP_KEY)).flatten()
    }

    fn take_external_change(&mut self) -> bool {
        self.external.replace(false)
    }

    fn read_current(&self) -> Option<String> {
        self.kv.get(CURRENT_KEY).filter(|text| text.len() <= MAX_SET_BYTES)
    }

    fn read_current_bytes(&self) -> Option<Vec<u8>> {
        self.kv.get(CURRENT_KEY).map(String::into_bytes)
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        within(json)?;
        let text = if self.kv.get(BACKUP_KEY).as_deref() == Some(json) { json.to_string() } else { self.stamped(json) };
        let mut index = self.read_index();
        let before = index.len();
        let written = self.set_evicting(CURRENT_KEY, &text, &mut index, None);
        if index.len() != before {
            // Best effort: a stale entry only makes a dropped snapshot unreadable.
            let _ = self.kv.set(INDEX_KEY, &index_json(&index));
        }
        written
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        within(json)?;
        let text = self.stamped(json);
        let key = snapshot_key(revision);
        let mut index = self.read_index();
        self.set_evicting(&key, &text, &mut index, Some(revision))?;
        if !index.contains(&revision) {
            index.push(revision);
            index.sort_unstable();
        }
        let expired = expired_snapshots(&index);
        index.retain(|kept| !expired.contains(kept));
        if let Err(error) = self.save_index(&mut index, Some(revision)) {
            self.kv.remove(&key);
            return Err(error);
        }
        for revision in expired {
            self.kv.remove(&snapshot_key(revision));
        }
        Ok(())
    }

    fn snapshots(&self) -> Vec<u64> {
        self.read_index()
    }

    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.kv.get(&snapshot_key(revision)).filter(|text| text.len() <= MAX_SET_BYTES + 1)
    }

    fn keep_damaged(&mut self, raw: &[u8]) -> Result<String, String> {
        let text = std::str::from_utf8(raw).map_err(|_| "the stored workspace is not UTF-8 text".to_string())?;
        let mut ids = self.damaged_ids();
        if let Some(same) = ids.iter().copied().find(|id| self.kv.get(&damaged_key(*id)).as_deref() == Some(text)) {
            return Ok(damaged_key(same));
        }
        let id = ids.last().map_or(0, |last| last.saturating_add(1));
        let key = damaged_key(id);
        // Only older damaged copies make room; snapshots and the current set are not touched.
        while let Err(error) = self.kv.set(&key, text) {
            if !self.drop_oldest_damaged(&mut ids) {
                return Err(error);
            }
        }
        ids.push(id);
        while ids.len() > MAX_DAMAGED_COPIES {
            self.drop_oldest_damaged(&mut ids);
        }
        while let Err(error) = self.kv.set(DAMAGED_INDEX_KEY, &index_json(&ids)) {
            if ids.len() <= 1 {
                self.kv.remove(&key);
                return Err(error);
            }
            self.drop_oldest_damaged(&mut ids);
        }
        Ok(key)
    }
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserStorage(web_sys::Storage);

#[cfg(target_arch = "wasm32")]
impl KeyValue for BrowserStorage {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get_item(key).ok().flatten()
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.0.set_item(key, value).map_err(|error| {
            log::warn!("workspace: localStorage refused {key}: {error:?}");
            "browser storage is full or blocked".to_string()
        })
    }

    fn remove(&mut self, key: &str) {
        let _ = self.0.remove_item(key);
    }
}

/// The page's `localStorage`; `None` when the browser blocks it.
#[cfg(target_arch = "wasm32")]
pub fn open() -> Option<LocalStorageStore<BrowserStorage>> {
    use wasm_bindgen::{JsCast, closure::Closure};

    let window = web_sys::window()?;
    let storage = window.local_storage().ok().flatten()?;
    let store = LocalStorageStore::new(BrowserStorage(storage), web_sys::js_sys::Date::now);
    let external = Rc::clone(&store.external);
    let repaint = Rc::clone(&store.repaint);
    let listener = Closure::<dyn FnMut(web_sys::StorageEvent)>::new(move |event: web_sys::StorageEvent| {
        if is_profile_key(event.key().as_deref()) {
            external.set(true);
            if let Ok(repaint) = repaint.try_borrow()
                && let Some(ctx) = repaint.as_ref()
            {
                ctx.request_repaint();
            }
        }
    });
    if let Err(error) = window.add_event_listener_with_callback("storage", listener.as_ref().unchecked_ref()) {
        log::warn!("workspace: could not listen for storage changes: {error:?}");
        return Some(store);
    }
    // The window owns this listener for the page's lifetime, after open returns.
    listener.forget();
    Some(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_ui_egui::wsprofile::startup::{Fallback, restore};
    use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES, ProfileStore};
    use std::collections::BTreeMap;

    #[test]
    fn storage_event_for_our_key_marks_external_change() {
        let mut store = store(FakeStorage::default());
        for (key, expected) in [(Some(CURRENT_KEY), true), (None, true), (Some(INDEX_KEY), false), (Some("unrelated"), false)] {
            if is_profile_key(key) {
                store.external.set(true);
            }
            assert_eq!(store.take_external_change(), expected, "{key:?}");
            assert!(!store.take_external_change(), "the notification is consumed once");
        }
    }

    /// A browser's storage: an optional byte quota, as `localStorage` has.
    #[derive(Default)]
    pub struct FakeStorage {
        items: BTreeMap<String, String>,
        quota: Option<usize>,
        blocked: bool,
    }

    impl FakeStorage {
        fn used(&self) -> usize {
            self.items.iter().map(|(key, value)| key.len() + value.len()).sum()
        }
    }

    impl KeyValue for FakeStorage {
        fn get(&self, key: &str) -> Option<String> {
            self.items.get(key).cloned()
        }
        fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
            if self.blocked {
                return Err("blocked".into());
            }
            let old = self.items.get(key).map_or(0, |old| key.len() + old.len());
            if self.quota.is_some_and(|quota| self.used() - old + key.len() + value.len() > quota) {
                return Err("quota".into());
            }
            self.items.insert(key.into(), value.into());
            Ok(())
        }
        fn remove(&mut self, key: &str) {
            self.items.remove(key);
        }
    }

    fn store(storage: FakeStorage) -> LocalStorageStore<FakeStorage> {
        LocalStorageStore::new(storage, || 1_800_000_000_000.0)
    }

    #[test]
    fn local_storage_backup_key_round_trips() {
        let mut store = store(FakeStorage::default());
        let raw = "{\r\n  \"schema_version\": \"1.0.0\"\r\n}";
        assert_eq!(store.read_backup("1.0.0"), None);
        store.keep_backup("1.0.0", raw).unwrap();
        store.keep_backup("1.0.0", "replacement").unwrap();
        assert_eq!(store.read_backup("1.0.0").as_deref(), Some(raw));
        assert_eq!(store.storage().get("cadcraft.ws.backup:1.0.0").as_deref(), Some(raw));
        assert_eq!(store.read_backup("4"), None);
        store.write_current(raw).unwrap();
        assert_eq!(store.read_current().as_deref(), Some(raw));
    }

    #[test]
    fn wire_clock_stamping_preserves_the_checksum_and_backup_bytes() {
        use cadcraft_ui_egui::wsprofile::canonical::{ChecksumCheck, verify_checksum, with_checksum};
        let raw = with_checksum(serde_json::json!({"schemaVersion": 3, "user": {
            "schemaVersion": 3, "profileId": "u", "createdAt": "1970-01-01T00:00:00Z", "updatedAt": "1970-01-01T00:00:00Z"
        }}))
        .to_string();
        let mut store = store(FakeStorage::default());
        store.keep_backup("1.0.0", &raw).unwrap();
        store.push_snapshot(1, &raw).unwrap();
        let stamped: Value = serde_json::from_str(&store.read_snapshot(1).unwrap()).unwrap();
        assert_eq!(stamped["user"]["updatedAt"], "2027-01-15T08:00:00Z");
        assert_eq!(verify_checksum(&stamped), ChecksumCheck::Valid);
        assert_eq!(store.read_backup("1.0.0").as_deref(), Some(raw.as_str()));
    }

    #[test]
    fn keys_follow_the_agreed_names() {
        assert_eq!(CURRENT_KEY, "cadcraft.ws.current");
        assert_eq!(INDEX_KEY, "cadcraft.ws.snaps");
        assert_eq!(snapshot_key(0), "cadcraft.ws.snap:0");
        assert_eq!(snapshot_key(17), "cadcraft.ws.snap:17");
        assert_eq!(DAMAGED_INDEX_KEY, "cadcraft.ws.damaged");
        assert_eq!(damaged_key(2), "cadcraft.ws.damaged:2");
    }

    #[test]
    fn the_snapshot_index_survives_storage_and_ignores_junk() {
        assert_eq!(parse_index(&index_json(&[3, 1, 2, 2])), [1, 2, 3]);
        assert_eq!(parse_index("not json"), Vec::<u64>::new());
        assert_eq!(parse_index(r#"{"a":1}"#), Vec::<u64>::new());
        assert_eq!(parse_index(r#"[5,"x",-1,2.5,null,4]"#), [4, 5]);
        let many: Vec<u64> = (0..1000).collect();
        assert!(parse_index(&index_json(&many)).len() <= KEEP_SNAPSHOTS + 1);
    }

    #[test]
    fn keeps_only_the_newest_snapshots_and_the_damaged_copy() {
        let mut store = store(FakeStorage::default());
        store.push_snapshot(CORRUPT_SNAPSHOT, "{damaged").unwrap();
        for revision in 1..=25u64 {
            store.push_snapshot(revision, &format!(r#"{{"rev":{revision}}}"#)).unwrap();
        }
        let kept = store.snapshots();
        assert_eq!(kept.len(), KEEP_SNAPSHOTS + 1, "{kept:?}");
        assert_eq!(kept.first(), Some(&CORRUPT_SNAPSHOT));
        assert_eq!(kept.get(1), Some(&6));
        assert_eq!(kept.last(), Some(&25));
        assert_eq!(store.read_snapshot(25).as_deref(), Some(r#"{"rev":25}"#));
        assert_eq!(store.read_snapshot(0).as_deref(), Some("{damaged"));
        assert_eq!(store.read_snapshot(5), None);
        let snapshot_keys = store.storage().items.keys().filter(|key| key.starts_with(SNAPSHOT_PREFIX)).count();
        assert_eq!(snapshot_keys, KEEP_SNAPSHOTS + 1, "pruned snapshots must not stay in storage");
    }

    #[test]
    fn current_round_trips() {
        let mut store = store(FakeStorage::default());
        assert_eq!(store.read_current(), None);
        store.write_current(r#"{"active":"SIMPLE"}"#).unwrap();
        assert_eq!(store.read_current().as_deref(), Some(r#"{"active":"SIMPLE"}"#));
        assert_eq!(store.read_policy(), None);
    }

    #[test]
    fn blocked_storage_is_an_error_not_a_panic() {
        let mut store = store(FakeStorage { blocked: true, ..FakeStorage::default() });
        assert!(store.write_current("{}").is_err());
        assert!(store.push_snapshot(1, "{}").is_err());
        assert_eq!(store.read_current(), None);
        assert!(store.snapshots().is_empty());
    }

    #[test]
    fn oversized_sets_are_refused() {
        let mut store = store(FakeStorage::default());
        let big = "x".repeat(MAX_SET_BYTES + 1);
        assert!(store.write_current(&big).is_err());
        assert!(store.push_snapshot(1, &big).is_err());
        assert!(store.storage().items.is_empty());
    }

    #[test]
    fn a_full_storage_gives_up_old_snapshots_before_the_current_set() {
        let payload = "p".repeat(1000);
        let mut store = store(FakeStorage { quota: Some(4500), ..FakeStorage::default() });
        for revision in 1..=4u64 {
            store.push_snapshot(revision, &payload).unwrap();
        }
        store.write_current(&payload).unwrap();
        // Full: the new revision displaces the oldest ones instead of failing the save.
        store.push_snapshot(5, &payload).unwrap();
        store.write_current(&format!("{payload}!")).unwrap();
        assert_eq!(store.read_current().map(|text| text.len()), Some(1001));
        let kept = store.snapshots();
        assert_eq!(kept.last(), Some(&5), "{kept:?}");
        assert!(!kept.contains(&1), "{kept:?}");
        assert!(store.read_snapshot(5).is_some());
    }

    #[test]
    fn a_set_that_cannot_fit_at_all_fails_cleanly() {
        let mut store = store(FakeStorage { quota: Some(100), ..FakeStorage::default() });
        assert!(store.write_current(&"x".repeat(500)).is_err());
        assert!(store.push_snapshot(1, &"x".repeat(500)).is_err());
        assert_eq!(store.read_current(), None);
    }

    #[test]
    fn stored_text_gets_the_browser_clock() {
        let stamp = "2027-01-15T08:00:00Z";
        let set = r#"{"active":"a","user":{"created_at":"1970-01-01T00:00:00Z","updated_at":"1970-01-01T00:00:00Z","name":"u"},"profiles":[{"created_at":"2026-10-21T00:00:00Z","updated_at":"1970-01-01T00:00:00Z"}]}"#;
        let stamped: serde_json::Value = serde_json::from_str(&stamp_clock(set, stamp, &mut Default::default())).unwrap();
        assert_eq!(stamped["user"]["created_at"], stamp);
        assert_eq!(stamped["user"]["updated_at"], stamp);
        assert_eq!(stamped["user"]["name"], "u");
        assert_eq!(stamped["profiles"][0]["created_at"], "2026-10-21T00:00:00Z");
        assert_eq!(stamped["profiles"][0]["updated_at"], stamp);
        // Text that is not a set, or already has real times, is left alone.
        assert_eq!(stamp_clock("{", stamp, &mut Default::default()), "{");
        let real = r#"{"user":{"updated_at":"2026-10-21T00:00:00Z"}}"#;
        assert_eq!(stamp_clock(real, stamp, &mut Default::default()), real);
    }

    #[test]
    fn the_store_stamps_what_it_writes() {
        let mut store = store(FakeStorage::default());
        let set = r#"{"user":{"updated_at":"1970-01-01T00:00:00Z"}}"#;
        store.write_current(set).unwrap();
        store.push_snapshot(1, set).unwrap();
        for text in [store.read_current().unwrap(), store.read_snapshot(1).unwrap()] {
            assert!(!text.contains("1970-"), "{text}");
            assert!(text.contains("2027-01-15T08:00:00Z"), "{text}");
        }
    }

    #[test]
    fn a_damaged_set_falls_back_and_keeps_the_copy() {
        let mut storage = FakeStorage::default();
        storage.items.insert(CURRENT_KEY.into(), "{".into());
        let mut store = store(storage);
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert_eq!(store.storage().get(&damaged_key(0)).as_deref(), Some("{"));
        assert!(store.snapshots().is_empty(), "a damaged copy is not a snapshot");
        assert_eq!(store.read_current().as_deref(), Some("{"), "the stored text stays until the next commit");
    }

    #[test]
    fn an_oversized_stored_set_is_handed_over_as_damaged() {
        let mut storage = FakeStorage::default();
        storage.items.insert(CURRENT_KEY.into(), "x".repeat(MAX_SET_BYTES * 2));
        let mut store = store(storage);
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert_eq!(store.storage().get(&damaged_key(0)).map(|text| text.len()), Some(MAX_SET_BYTES * 2));
        assert_eq!(store.read_current().as_deref(), None);
    }

    #[test]
    fn a_working_app_round_trips_through_the_store() {
        use cadcraft_ui_egui::CadApp;
        let new_app = || CadApp::new(cadcraft_engine::Session::default(), Default::default());
        let mut app = new_app();
        app.attach_profile_store(Box::new(store(FakeStorage::default())));
        app.run("ui.workspace.set", serde_json::json!({"pointer": "/ui_scale", "value": 1.5})).unwrap();
        app.commit_workspace_now().unwrap();
        let current = app.ws.store.as_ref().and_then(|stored| stored.read_current()).unwrap();
        let mut again = new_app();
        let mut fresh = store(FakeStorage::default());
        fresh.write_current(&current).unwrap();
        again.attach_profile_store(Box::new(fresh));
        assert_eq!(again.ws.device.ui_scale, 1.5);
        assert!(again.ws.fallback.is_none(), "{:?}", again.ws.fallback);
    }

    static CLOCK_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1_800_000_000_000);

    fn ticking_clock() -> f64 {
        CLOCK_MS.load(std::sync::atomic::Ordering::Relaxed) as f64
    }

    #[test]
    fn created_at_is_stamped_once_and_stays() {
        let mut store = LocalStorageStore::new(FakeStorage::default(), ticking_clock);
        let set = |extra: &str| {
            format!(r#"{{"user":{{"profile_id":"u","created_at":"1970-01-01T00:00:00Z","updated_at":"1970-01-01T00:00:00Z"}},"profiles":[{extra}]}}"#)
        };
        let read = |store: &LocalStorageStore<FakeStorage>| serde_json::from_str::<serde_json::Value>(&store.read_current().unwrap()).unwrap();
        store.write_current(&set("")).unwrap();
        let first = read(&store);
        CLOCK_MS.fetch_add(3_600_000, std::sync::atomic::Ordering::Relaxed);
        let later = r#"{"profile_id":"p","created_at":"1970-01-01T00:00:00Z","updated_at":"1970-01-01T00:00:00Z"}"#;
        store.write_current(&set(later)).unwrap();
        let second = read(&store);
        assert_eq!(second["user"]["created_at"], first["user"]["created_at"], "created_at moved on a later save");
        assert_ne!(second["user"]["updated_at"], first["user"]["updated_at"]);
        assert_eq!(second["profiles"][0]["created_at"], second["user"]["updated_at"], "a profile first seen later is created then");
        CLOCK_MS.fetch_add(3_600_000, std::sync::atomic::Ordering::Relaxed);
        store.write_current(&set(later)).unwrap();
        assert_eq!(read(&store)["profiles"][0]["created_at"], second["profiles"][0]["created_at"]);
        // A new page load starts from what was stored.
        let mut reloaded = LocalStorageStore::new(FakeStorage { items: store.storage().items.clone(), ..FakeStorage::default() }, ticking_clock);
        CLOCK_MS.fetch_add(3_600_000, std::sync::atomic::Ordering::Relaxed);
        reloaded.write_current(&set("")).unwrap();
        assert_eq!(read(&reloaded)["user"]["created_at"], first["user"]["created_at"]);
    }

    #[test]
    fn the_index_update_gives_up_an_old_snapshot_instead_of_failing_the_commit() {
        let payload = "p".repeat(1000);
        let mut store = store(FakeStorage::default());
        for revision in 1..=3u64 {
            store.push_snapshot(revision, &payload).unwrap();
        }
        // Room for the next snapshot, but not for the longer index that lists it.
        let room = snapshot_key(4).len() + payload.len();
        let used = store.storage().used();
        store.kv.quota = Some(used + room);
        store.push_snapshot(4, &payload).unwrap();
        assert_eq!(store.snapshots(), [2, 3, 4]);
        assert!(store.read_snapshot(4).is_some() && store.read_snapshot(1).is_none());
    }

    #[test]
    fn a_second_damaged_copy_does_not_replace_the_first() {
        let mut store = store(FakeStorage::default());
        let first = store.keep_damaged(b"{first").unwrap();
        let second = store.keep_damaged(b"{second").unwrap();
        assert_ne!(first, second);
        assert_eq!(store.storage().get(&first).as_deref(), Some("{first"));
        assert_eq!(store.storage().get(&second).as_deref(), Some("{second"));
        assert_eq!(store.keep_damaged(b"{first").unwrap(), first, "the same text again adds no copy");
        assert!(store.snapshots().is_empty());
        assert!(store.keep_damaged(&[b'{', 0xff]).is_err(), "text that is not UTF-8 cannot be kept in localStorage");
    }

    #[test]
    fn only_the_newest_damaged_copies_are_kept_and_nothing_else_is_touched() {
        let mut store = store(FakeStorage::default());
        store.write_current("{}").unwrap();
        store.push_snapshot(1, "{}").unwrap();
        for n in 0..5 {
            store.keep_damaged(format!("{{{n}").as_bytes()).unwrap();
        }
        let kept: Vec<String> = store.storage().items.keys().filter(|key| key.starts_with(DAMAGED_PREFIX)).cloned().collect();
        assert_eq!(kept, [damaged_key(2), damaged_key(3), damaged_key(4)]);
        assert_eq!(store.storage().get(&damaged_key(4)).as_deref(), Some("{4"));
        assert_eq!(store.snapshots(), [1]);
        assert!(store.read_current().is_some());
    }

    #[test]
    fn a_full_storage_drops_old_damaged_copies_but_never_snapshots_or_current() {
        let mut store = store(FakeStorage::default());
        store.write_current("{}").unwrap();
        store.push_snapshot(1, "{}").unwrap();
        store.keep_damaged(&[b'a'; 1000]).unwrap();
        let used = store.storage().used();
        store.kv.quota = Some(used + 500);
        store.keep_damaged(&[b'b'; 1000]).unwrap();
        assert!(store.storage().get(&damaged_key(0)).is_none() && store.storage().get(&damaged_key(1)).is_some());
        assert_eq!(store.snapshots(), [1]);
        // Too big for any room: an error, with the rest intact.
        store.kv.quota = Some(store.storage().used() + 10);
        assert!(store.keep_damaged(&[b'c'; 5000]).is_err());
        assert!(store.read_current().is_some() && store.read_snapshot(1).is_some());
    }

    #[test]
    fn startup_keeps_a_damaged_set_without_touching_snapshots() {
        let mut storage = FakeStorage::default();
        storage.items.insert(CURRENT_KEY.into(), "{".into());
        let mut store = store(storage);
        restore(&mut store);
        let again = restore(&mut store);
        assert!(matches!(again.fallback, Some(Fallback::Corrupt(_))));
        assert_eq!(store.storage().items.keys().filter(|key| key.starts_with(DAMAGED_PREFIX)).count(), 1, "the same damage is kept once");
        assert!(store.snapshots().is_empty());
    }
}

//! The browser profile store: the workspace profile set and its snapshots in `localStorage`.
//! `localStorage` is a few MB for the whole site (fonts share it), so a full or blocked storage is
//! an `Err` the app shows, never a panic.

use cadcraft_ui_egui::wsprofile::rfc3339;
use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES, ProfileStore, expired_snapshots};
use serde_json::Value;

pub const CURRENT_KEY: &str = "cadcraft.ws.current";
pub const SNAPSHOT_PREFIX: &str = "cadcraft.ws.snap:";
/// The revisions kept, as a JSON list; `localStorage` cannot list keys by prefix cheaply.
pub const INDEX_KEY: &str = "cadcraft.ws.snaps";

/// The start of every time the core stamps while it has no clock (the web build has none).
const EPOCH_PREFIX: &str = "1970-01-01T00:00:00";

pub fn snapshot_key(revision: u64) -> String {
    format!("{SNAPSHOT_PREFIX}{revision}")
}

/// What the store needs from `localStorage`; a fake stands in for it in tests.
pub trait KeyValue {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: &str) -> Result<(), String>;
    fn remove(&mut self, key: &str);
}

/// Reads a saved index; anything unreadable is dropped, and a hostile one is cut to what the store keeps.
pub fn parse_index(text: &str) -> Vec<u64> {
    let list = serde_json::from_str::<Value>(text).ok().and_then(|value| value.as_array().cloned()).unwrap_or_default();
    let mut revisions: Vec<u64> = list.iter().filter_map(Value::as_u64).filter(|revision| *revision != CORRUPT_SNAPSHOT).collect();
    let damaged_copy = list.iter().any(|value| value.as_u64() == Some(CORRUPT_SNAPSHOT));
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

/// Replaces the 1970 times the core writes on the web (no clock there) with `now`.
pub fn stamp_clock(json: &str, now: &str) -> String {
    if !json.contains(EPOCH_PREFIX) {
        return json.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(json) else { return json.to_string() };
    let mut changed = false;
    let mut stamp = |profile: &mut Value| {
        for field in ["created_at", "updated_at"] {
            if let Some(slot) = profile.get_mut(field)
                && slot.as_str().is_some_and(|time| time.starts_with(EPOCH_PREFIX))
            {
                *slot = Value::String(now.to_string());
                changed = true;
            }
        }
    };
    if let Some(user) = value.get_mut("user") {
        stamp(user);
    }
    if let Some(profiles) = value.get_mut("profiles").and_then(Value::as_array_mut) {
        profiles.iter_mut().for_each(&mut stamp);
    }
    if !changed {
        return json.to_string();
    }
    serde_json::to_string(&value).unwrap_or_else(|_| json.to_string())
}

/// One byte past the cap, so an oversized text reaches startup as damaged instead of being cut silently.
fn capped(text: String) -> String {
    let mut end = MAX_SET_BYTES + 1;
    if text.len() <= end {
        return text;
    }
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_string()
}

fn within(json: &str, revision: Option<u64>) -> Result<(), String> {
    let limit = if revision == Some(CORRUPT_SNAPSHOT) { MAX_SET_BYTES + 1 } else { MAX_SET_BYTES };
    if json.len() > limit { Err(format!("the workspace is larger than {MAX_SET_BYTES} bytes")) } else { Ok(()) }
}

pub struct LocalStorageStore<K: KeyValue> {
    kv: K,
    now_ms: fn() -> f64,
}

impl<K: KeyValue> LocalStorageStore<K> {
    pub fn new(kv: K, now_ms: fn() -> f64) -> Self {
        Self { kv, now_ms }
    }

    #[cfg(test)]
    fn storage(&self) -> &K {
        &self.kv
    }

    fn read_index(&self) -> Vec<u64> {
        self.kv.get(INDEX_KEY).map(|text| parse_index(&text)).unwrap_or_default()
    }

    fn stamped(&self, json: &str) -> String {
        stamp_clock(json, &rfc3339((self.now_ms)()))
    }

    /// Stores `value`; when the storage is full, the oldest snapshots make room (the current set
    /// matters more than its history), one at a time, until it fits or none is left.
    fn set_evicting(&mut self, key: &str, value: &str) -> Result<(), String> {
        loop {
            let error = match self.kv.set(key, value) {
                Ok(()) => return Ok(()),
                Err(error) => error,
            };
            let mut index = self.read_index();
            let Some(oldest) = index.iter().copied().find(|revision| *revision != CORRUPT_SNAPSHOT) else { return Err(error) };
            log::warn!("workspace: browser storage is full ({error}); dropping snapshot {oldest}");
            index.retain(|revision| *revision != oldest);
            self.kv.remove(&snapshot_key(oldest));
            // Best effort: a stale entry only makes a missing snapshot unreadable.
            let _ = self.kv.set(INDEX_KEY, &index_json(&index));
        }
    }
}

impl<K: KeyValue> ProfileStore for LocalStorageStore<K> {
    fn read_current(&self) -> Option<String> {
        self.kv.get(CURRENT_KEY).map(capped)
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        within(json, None)?;
        let text = self.stamped(json);
        self.set_evicting(CURRENT_KEY, &text)
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        within(json, Some(revision))?;
        let text = self.stamped(json);
        let key = snapshot_key(revision);
        self.set_evicting(&key, &text)?;
        let mut index = self.read_index();
        if !index.contains(&revision) {
            index.push(revision);
            index.sort_unstable();
        }
        let expired = expired_snapshots(&index);
        index.retain(|kept| !expired.contains(kept));
        if let Err(error) = self.kv.set(INDEX_KEY, &index_json(&index)) {
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
        self.kv.get(&snapshot_key(revision)).map(capped)
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
    let storage = web_sys::window()?.local_storage().ok().flatten()?;
    Some(LocalStorageStore::new(BrowserStorage(storage), web_sys::js_sys::Date::now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadcraft_ui_egui::wsprofile::startup::{Fallback, restore};
    use cadcraft_ui_egui::wsprofile::store::{CORRUPT_SNAPSHOT, KEEP_SNAPSHOTS, MAX_SET_BYTES, ProfileStore};
    use std::collections::BTreeMap;

    /// A browser's storage: an optional byte quota, as `localStorage` has.
    #[derive(Default)]
    struct FakeStorage {
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
    fn keys_follow_the_agreed_names() {
        assert_eq!(CURRENT_KEY, "cadcraft.ws.current");
        assert_eq!(INDEX_KEY, "cadcraft.ws.snaps");
        assert_eq!(snapshot_key(0), "cadcraft.ws.snap:0");
        assert_eq!(snapshot_key(17), "cadcraft.ws.snap:17");
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
        let stamped: serde_json::Value = serde_json::from_str(&stamp_clock(set, stamp)).unwrap();
        assert_eq!(stamped["user"]["created_at"], stamp);
        assert_eq!(stamped["user"]["updated_at"], stamp);
        assert_eq!(stamped["user"]["name"], "u");
        assert_eq!(stamped["profiles"][0]["created_at"], "2026-10-21T00:00:00Z");
        assert_eq!(stamped["profiles"][0]["updated_at"], stamp);
        // Text that is not a set, or already has real times, is left alone.
        assert_eq!(stamp_clock("{", stamp), "{");
        let real = r#"{"user":{"updated_at":"2026-10-21T00:00:00Z"}}"#;
        assert_eq!(stamp_clock(real, stamp), real);
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
        assert_eq!(store.read_snapshot(CORRUPT_SNAPSHOT).as_deref(), Some("{"));
        assert_eq!(store.read_current().as_deref(), Some("{"), "the stored text stays until the next commit");
    }

    #[test]
    fn an_oversized_stored_set_is_handed_over_as_damaged() {
        let mut storage = FakeStorage::default();
        storage.items.insert(CURRENT_KEY.into(), "x".repeat(MAX_SET_BYTES * 2));
        let mut store = store(storage);
        let restored = restore(&mut store);
        assert!(matches!(restored.fallback, Some(Fallback::Corrupt(_))), "{:?}", restored.fallback);
        assert!(store.read_snapshot(CORRUPT_SNAPSHOT).is_some());
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
        assert_eq!(again.ws.user.ui_scale, 1.5);
        assert!(again.ws.fallback.is_none(), "{:?}", again.ws.fallback);
    }
}

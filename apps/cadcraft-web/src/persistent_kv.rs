use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::job_store::JobKv;

const MAX_RECORDS: usize = crate::idb_profile_store::MAX_PRELOAD_RECORDS;
const MAX_BYTES: usize = 32 << 20;
const MAX_WRITES: usize = 64;

type Entries = Vec<(String, Arc<Vec<u8>>)>;

#[derive(Default)]
struct State {
    mirror: BTreeMap<String, Arc<Vec<u8>>>,
    durable: BTreeMap<String, Arc<Vec<u8>>>,
    writes: VecDeque<Entries>,
    queued_bytes: usize,
    error: Option<String>,
}

#[derive(Clone, Default)]
pub struct CachedKv(Arc<Mutex<State>>);

impl CachedKv {
    fn lock(&self) -> Result<MutexGuard<'_, State>, String> {
        self.0.lock().map_err(|_| "print storage cache is unavailable".into())
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn preload(entries: Vec<(String, Vec<u8>)>) -> Result<Self, String> {
        if entries.len() > MAX_RECORDS {
            return Err("print storage exceeds its record limit".into());
        }
        let bytes = entries.iter().try_fold(0usize, |total, (key, bytes)| total.checked_add(key.len())?.checked_add(bytes.len()));
        if bytes.is_none_or(|bytes| bytes > MAX_BYTES) {
            return Err("print storage exceeds its byte limit".into());
        }
        let mirror: BTreeMap<_, _> = entries.into_iter().map(|(key, bytes)| (key, Arc::new(bytes))).collect();
        Ok(Self(Arc::new(Mutex::new(State { durable: mirror.clone(), mirror, ..Default::default() }))))
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn acknowledge(&self, result: Result<(), String>) -> Result<(), String> {
        let mut state = self.lock()?;
        if let Err(error) = result {
            state.mirror = state.durable.clone();
            state.writes.clear();
            state.queued_bytes = 0;
            state.error = Some(error.clone());
            return Err(error);
        }
        if let Some(entries) = state.writes.pop_front() {
            for (key, bytes) in entries {
                state.queued_bytes = state.queued_bytes.saturating_sub(key.len().saturating_add(bytes.len()));
                state.durable.insert(key, bytes);
            }
        }
        Ok(())
    }
}

impl JobKv for CachedKv {
    fn put_many(&mut self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
        let mut state = self.lock()?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if entries.len() > MAX_RECORDS || state.writes.len() >= MAX_WRITES {
            return Err("print storage queue is full".into());
        }
        let added = entries
            .iter()
            .try_fold(0usize, |total, (key, bytes)| total.checked_add(key.len())?.checked_add(bytes.len()))
            .ok_or("print storage queue exceeds its byte limit")?;
        let queued_bytes = state.queued_bytes.checked_add(added).filter(|total| *total <= MAX_BYTES).ok_or("print storage queue is full")?;
        let mut mirror = state.mirror.clone();
        let entries: Entries = entries.iter().map(|(key, bytes)| (key.clone(), Arc::new(bytes.clone()))).collect();
        mirror.extend(entries.iter().cloned());
        let total = mirror.iter().try_fold(0usize, |total, (key, bytes)| total.checked_add(key.len())?.checked_add(bytes.len()));
        if mirror.len() > MAX_RECORDS || total.is_none_or(|total| total > MAX_BYTES) {
            return Err("print storage exceeds supported limits".into());
        }
        state.mirror = mirror;
        state.queued_bytes = queued_bytes;
        state.writes.push_back(entries);
        Ok(())
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.lock().ok()?.mirror.get(key).map(|bytes| bytes.as_ref().clone())
    }

    fn delete(&mut self, _key: &str) {
        if let Ok(mut state) = self.lock() {
            state.error = Some("print profile and template records cannot be deleted".into());
        }
    }

    fn keys(&self, prefix: &str) -> Vec<String> {
        self.lock().map(|state| state.mirror.keys().filter(|key| key.starts_with(prefix)).cloned().collect()).unwrap_or_default()
    }

    fn poll(&mut self) -> Result<bool, String> {
        let state = self.lock()?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        Ok(state.writes.is_empty())
    }
}

#[cfg(target_arch = "wasm32")]
pub struct BrowserWriter {
    idb: crate::idb::IdbStore,
    store: &'static str,
    kv: CachedKv,
    pending: Option<crate::idb::Pending>,
    failed: bool,
}

#[cfg(target_arch = "wasm32")]
impl BrowserWriter {
    pub async fn open(store: &'static str) -> Result<(CachedKv, Self), String> {
        let idb = crate::idb::IdbStore::open().await.map_err(|error| error.message())?;
        let entries = crate::idb::with_timeout(idb.preload(store, MAX_RECORDS)).await.map_err(|error| error.message())?;
        let kv = CachedKv::preload(entries)?;
        Ok((kv.clone(), Self { idb, store, kv, pending: None, failed: false }))
    }

    pub fn store_name(&self) -> &'static str {
        self.store
    }

    pub fn cache(&self) -> CachedKv {
        self.kv.clone()
    }

    pub fn poll(&mut self) -> Result<bool, String> {
        if self.failed {
            return Ok(true);
        }
        let result = self.poll_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn poll_inner(&mut self) -> Result<bool, String> {
        for _ in 0..MAX_WRITES {
            if let Some(pending) = &self.pending {
                let Some(result) = pending.result() else { return Ok(false) };
                self.pending = None;
                self.kv.acknowledge(result.map_err(|error| error.message()))?;
            }
            // Only one transaction may be in flight, so a failed batch restores the exact durable cache.
            let entries = self.kv.lock()?.writes.front().cloned();
            let Some(entries) = entries else { return Ok(true) };
            let entries: Vec<_> = entries.into_iter().map(|(key, bytes)| (key, bytes.as_ref().clone())).collect();
            self.pending = Some(self.idb.put_many(self.store, &entries));
        }
        Ok(false)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub mod testing {
    use super::*;

    #[derive(Default)]
    struct FakeState {
        entries: BTreeMap<String, Vec<u8>>,
        fail_writes: bool,
    }

    #[derive(Clone, Default)]
    pub struct FakeKv(Arc<Mutex<FakeState>>);

    impl FakeKv {
        pub fn reload(&self) -> Self {
            Self(Arc::new(Mutex::new(FakeState { entries: self.0.lock().unwrap().entries.clone(), fail_writes: false })))
        }

        pub fn fail_writes(&mut self) {
            self.0.lock().unwrap().fail_writes = true;
        }
    }

    impl JobKv for FakeKv {
        fn put_many(&mut self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            if state.fail_writes {
                return Err("QuotaExceededError".into());
            }
            state.entries.extend(entries.iter().cloned());
            Ok(())
        }

        fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().entries.get(key).cloned()
        }

        fn delete(&mut self, key: &str) {
            self.0.lock().unwrap().entries.remove(key);
        }

        fn keys(&self, prefix: &str) -> Vec<String> {
            self.0.lock().unwrap().entries.keys().filter(|key| key.starts_with(prefix)).cloned().collect()
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn stores_reject_writes_after_failed_async_commit() {
        use cadcraft_engine::sheetplan::profile::{PrintProfileStore, ProfileLevel};
        use cadcraft_engine::sheetplan::registry::FrameRegistry;
        use cadcraft_engine::sheetplan::registry_store::{RegistryStore, save_registry};

        let kv = CachedKv::default();
        let mut profiles = crate::print_profile_store::IdbPrintProfileStore::new(kv.clone());
        let json = serde_json::json!({
            "schema_version": 1, "level": "user", "id": "default", "revision": 1, "settings": {}, "updated_ms": 0
        })
        .to_string();
        profiles.write(ProfileLevel::User, "default", &json).unwrap();
        kv.acknowledge(Ok(())).unwrap();
        let changed = json.replace("\"revision\":1", "\"revision\":2");
        profiles.write(ProfileLevel::User, "default", &changed).unwrap();
        assert!(kv.acknowledge(Err("QuotaExceededError".into())).is_err());
        assert_eq!(profiles.read(ProfileLevel::User, "default").unwrap(), Some(json.as_bytes().to_vec()));
        assert_eq!(profiles.write(ProfileLevel::User, "default", &json), Err("QuotaExceededError".into()));
        assert_eq!(profiles.keep_damaged(ProfileLevel::User, "default", b"{"), Err("QuotaExceededError".into()));

        let kv = CachedKv::default();
        let mut registry = crate::registry_store::IdbRegistryStore::new(kv.clone());
        let json = save_registry(&FrameRegistry::builtin());
        registry.write(&json).unwrap();
        kv.acknowledge(Ok(())).unwrap();
        registry.write(&json).unwrap();
        assert!(kv.acknowledge(Err("QuotaExceededError".into())).is_err());
        assert_eq!(registry.read().unwrap(), Some(json.clone()));
        assert_eq!(registry.read_last_good().unwrap(), None);
        assert_eq!(registry.write(&json), Err("QuotaExceededError".into()));
    }

    #[test]
    fn failed_commit_restores_durable_cache_and_latches_error() {
        let mut kv = CachedKv::preload(vec![("current".into(), b"old".to_vec())]).unwrap();
        kv.put_many(&[("current".into(), b"committed".to_vec())]).unwrap();
        assert!(!kv.poll().unwrap());
        kv.acknowledge(Ok(())).unwrap();
        assert!(kv.poll().unwrap());
        kv.put_many(&[("current".into(), b"failed".to_vec())]).unwrap();
        kv.put_many(&[("later".into(), b"queued".to_vec())]).unwrap();
        assert!(kv.acknowledge(Err("QuotaExceededError".into())).is_err());
        assert_eq!(kv.get("current"), Some(b"committed".to_vec()));
        assert_eq!(kv.get("later"), None);
        assert_eq!(kv.poll(), Err("QuotaExceededError".into()));
        assert_eq!(kv.put_many(&[("current".into(), b"retry".to_vec())]), Err("QuotaExceededError".into()));
    }
}

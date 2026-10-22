use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::sheetplan::registry::FrameRegistry;
use cadcraft_engine::sheetplan::registry_store::RegistryStore;

use crate::job_store::JobKv;

const CURRENT: &str = "registry/current";
const LAST_GOOD: &str = "registry/last-good";
const MAX_BYTES: usize = 2 << 20;

pub struct IdbRegistryStore<K: JobKv + Send> {
    kv: Mutex<K>,
}

impl<K: JobKv + Send> IdbRegistryStore<K> {
    pub fn new(kv: K) -> Self {
        Self { kv: Mutex::new(kv) }
    }

    fn storage(&self) -> Result<MutexGuard<'_, K>, String> {
        self.kv.lock().map_err(|_| "frame template cache is unavailable".into())
    }

    fn read_key(&self, key: &str) -> Result<Option<String>, String> {
        self.storage()?
            .get(key)
            .map(|bytes| {
                if bytes.len() > MAX_BYTES {
                    return Err("frame template registry exceeds 2 MiB".into());
                }
                String::from_utf8(bytes).map_err(|error| error.to_string())
            })
            .transpose()
    }
}

impl<K: JobKv + Send> RegistryStore for IdbRegistryStore<K> {
    fn read(&self) -> Result<Option<String>, String> {
        self.read_key(CURRENT)
    }

    fn read_last_good(&self) -> Result<Option<String>, String> {
        self.read_key(LAST_GOOD)
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        FrameRegistry::from_json(text)?;
        let mut kv = self.storage()?;
        kv.poll()?;
        let previous = kv
            .get(CURRENT)
            .filter(|bytes| bytes.len() <= MAX_BYTES)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .filter(|previous| FrameRegistry::from_json(previous).is_ok());
        let mut entries = Vec::new();
        if let Some(previous) = previous {
            entries.push((LAST_GOOD.into(), previous.into_bytes()));
        }
        entries.push((CURRENT.into(), text.as_bytes().to_vec()));
        kv.put_many(&entries)
    }
}

#[cfg(target_arch = "wasm32")]
pub fn install_host(kv: crate::persistent_kv::CachedKv) -> Result<(), String> {
    use cadcraft_engine::sheetplan::registry_store::set_registry_store_factory;

    use crate::persistent_kv::CachedKv;

    static HOST: Mutex<Option<CachedKv>> = Mutex::new(None);

    fn open() -> Result<Box<dyn RegistryStore>, String> {
        let kv = HOST.lock().map_err(|_| "frame template cache is unavailable")?.clone().ok_or("frame template cache was not preloaded")?;
        Ok(Box::new(IdbRegistryStore::new(kv)))
    }

    *HOST.lock().map_err(|_| "frame template cache is unavailable")? = Some(kv);
    set_registry_store_factory(open);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use cadcraft_engine::sheetplan::registry_store::{load_stored_registry, save_registry};

    use super::*;
    use crate::persistent_kv::testing::FakeKv;

    fn registry() -> String {
        let mut registry = FrameRegistry::builtin();
        registry.templates.truncate(1);
        save_registry(&registry)
    }

    #[test]
    fn registry_survives_reload() {
        let kv = FakeKv::default();
        let mut store = IdbRegistryStore::new(kv.clone());
        let first = registry();
        let mut next = FrameRegistry::builtin();
        let old = next.templates.first_mut().unwrap();
        let mut successor = old.clone();
        successor.version = 2;
        successor.revision = "synthetic revision".into();
        successor.source = Some(cadcraft_engine::sheetplan::registry::TemplateSource {
            file_name: "synthetic.dxf".into(),
            sha256: "a".repeat(64),
            imported_ms: 123,
        });
        old.status = cadcraft_engine::sheetplan::registry::TemplateStatus::Superseded;
        old.superseded_by = Some(successor.versioned_id());
        next.templates.push(successor);
        let second = save_registry(&next);
        store.write(&first).unwrap();
        store.write(&second).unwrap();
        let reloaded = IdbRegistryStore::new(kv.reload());
        assert_eq!(reloaded.read().unwrap(), Some(second.clone()));
        assert_eq!(reloaded.read_last_good().unwrap(), Some(first));
        let (loaded, notice) = load_stored_registry(&reloaded);
        assert_eq!(save_registry(&loaded), second);
        assert!(notice.is_none());
    }

    #[test]
    fn damaged_registry_restores_last_good_without_overwriting_current() {
        let mut kv = FakeKv::default();
        let mut store = IdbRegistryStore::new(kv.clone());
        let first = registry();
        store.write(&first).unwrap();
        store.write(&save_registry(&FrameRegistry::builtin())).unwrap();
        kv.put_many(&[(CURRENT.into(), b"{".to_vec())]).unwrap();
        let reloaded = IdbRegistryStore::new(kv.reload());
        let (loaded, notice) = load_stored_registry(&reloaded);
        assert_eq!(save_registry(&loaded), first);
        assert!(notice.unwrap().contains("restored the last good copy"));
        assert_eq!(reloaded.read().unwrap(), Some("{".into()));
    }

    #[test]
    fn registry_write_error_keeps_current_and_last_good() {
        let mut kv = FakeKv::default();
        let mut store = IdbRegistryStore::new(kv.clone());
        let first = registry();
        store.write(&first).unwrap();
        kv.fail_writes();
        assert_eq!(store.write(&save_registry(&FrameRegistry::builtin())), Err("QuotaExceededError".into()));
        assert_eq!(store.read().unwrap(), Some(first));
        assert_eq!(store.read_last_good().unwrap(), None);
    }
}

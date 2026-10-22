use std::sync::{Mutex, MutexGuard};

use cadcraft_engine::hash::sha256_hex;
use cadcraft_engine::sheetplan::profile::{MAX_PROFILE_BYTES, PrintProfileStore, ProfileLevel, migrate};

use crate::job_store::JobKv;

const MAX_PROFILES: usize = 1024;
const MAX_DAMAGED: usize = 256;

pub struct IdbPrintProfileStore<K: JobKv + Send> {
    kv: Mutex<K>,
}

fn profile_key(level: ProfileLevel, id: &str) -> Result<String, String> {
    let drawing_name = level == ProfileLevel::Drawing
        && id
            .strip_prefix("name:")
            .is_some_and(|hash| hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()));
    if level == ProfileLevel::Platform
        || (!drawing_name
            && (id.is_empty()
                || id.len() > 64
                || !id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                || (level == ProfileLevel::Drawing && id.starts_with("name--"))))
    {
        return Err("invalid print profile store key".into());
    }
    let level = match level {
        ProfileLevel::Platform => "platform",
        ProfileLevel::Company => "company",
        ProfileLevel::Project => "project",
        ProfileLevel::SheetSet => "sheet_set",
        ProfileLevel::Drawing => "drawing",
        ProfileLevel::User => "user",
    };
    Ok(format!("{level}/{id}"))
}

fn valid_profile(raw: &[u8], level: ProfileLevel, id: &str) -> Result<(), String> {
    if raw.len() > MAX_PROFILE_BYTES {
        return Err("print profile exceeds 256 KiB".into());
    }
    let doc = migrate(serde_json::from_slice(raw).map_err(|error| error.to_string())?).map_err(|error| error.to_string())?;
    if doc.level != level || doc.id != id {
        return Err("print profile level or id does not match its store key".into());
    }
    Ok(())
}

fn damaged_entry<K: JobKv>(kv: &K, key: &str, raw: &[u8]) -> Result<(String, Vec<u8>), String> {
    if raw.len() > MAX_PROFILE_BYTES {
        return Err("damaged print profile exceeds 256 KiB".into());
    }
    let prefix = format!("{key}/damaged/");
    let damaged_key = format!("{prefix}{}", sha256_hex(raw));
    if kv.get(&damaged_key).is_none() && kv.keys(&prefix).len() >= MAX_DAMAGED {
        return Err("damaged print profile count limit reached".into());
    }
    Ok((damaged_key, raw.to_vec()))
}

impl<K: JobKv + Send> IdbPrintProfileStore<K> {
    pub fn new(kv: K) -> Self {
        Self { kv: Mutex::new(kv) }
    }

    fn storage(&self) -> Result<MutexGuard<'_, K>, String> {
        self.kv.lock().map_err(|_| "print profile cache is unavailable".into())
    }
}

impl<K: JobKv + Send> PrintProfileStore for IdbPrintProfileStore<K> {
    fn read(&self, level: ProfileLevel, id: &str) -> Result<Option<Vec<u8>>, String> {
        let key = profile_key(level, id)?;
        Ok(self.storage()?.get(&key))
    }

    fn write(&mut self, level: ProfileLevel, id: &str, json: &str) -> Result<(), String> {
        let key = profile_key(level, id)?;
        valid_profile(json.as_bytes(), level, id)?;
        let mut kv = self.storage()?;
        kv.poll()?;
        if kv.keys("").into_iter().filter(|key| key.split('/').count() == 2).count() >= MAX_PROFILES && kv.get(&key).is_none() {
            return Err("print profile store count limit reached".into());
        }
        let good_key = format!("{key}/last-good");
        let mut entries = Vec::new();
        match kv.get(&key) {
            Some(previous) if valid_profile(&previous, level, id).is_ok() => entries.push((good_key, previous)),
            previous => {
                if let Some(previous) = previous {
                    entries.push(damaged_entry(&*kv, &key, &previous)?);
                }
                if kv.get(&good_key).is_none() {
                    entries.push((good_key, json.as_bytes().to_vec()));
                }
            }
        }
        entries.push((key, json.as_bytes().to_vec()));
        kv.put_many(&entries)
    }

    fn read_last_good(&self, level: ProfileLevel, id: &str) -> Option<String> {
        let key = profile_key(level, id).ok()?;
        String::from_utf8(self.storage().ok()?.get(&format!("{key}/last-good"))?).ok()
    }

    fn keep_damaged(&mut self, level: ProfileLevel, id: &str, raw: &[u8]) -> Result<(), String> {
        let key = profile_key(level, id)?;
        let mut kv = self.storage()?;
        kv.poll()?;
        let entry = damaged_entry(&*kv, &key, raw)?;
        kv.put_many(&[entry])
    }

    fn list(&self, level: ProfileLevel) -> Vec<String> {
        let Ok(key) = profile_key(level, "default") else { return Vec::new() };
        let Some((level_key, _)) = key.split_once('/') else { return Vec::new() };
        let prefix = format!("{level_key}/");
        let Ok(kv) = self.storage() else { return Vec::new() };
        kv.keys(&prefix)
            .into_iter()
            .filter_map(|key| key.strip_prefix(&prefix).filter(|id| profile_key(level, id).is_ok()).map(str::to_owned))
            .take(MAX_PROFILES)
            .collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use cadcraft_engine::sheetplan::profile::load_doc;
    use serde_json::json;

    use super::*;
    use crate::persistent_kv::testing::FakeKv;

    fn profile(revision: u64) -> String {
        json!({"schema_version": 1, "level": "user", "id": "default", "revision": revision, "settings": {}, "updated_ms": 0}).to_string()
    }

    #[test]
    fn idb_print_profile_round_trip() {
        let kv = FakeKv::default();
        let mut store = IdbPrintProfileStore::new(kv.clone());
        let levels = [ProfileLevel::Company, ProfileLevel::Project, ProfileLevel::SheetSet, ProfileLevel::Drawing, ProfileLevel::User];
        for level in levels {
            let id = if level == ProfileLevel::Drawing { "name:0123456789abcdef" } else { "default" };
            let text = json!({
                "schema_version": 1, "level": level, "id": id, "revision": 1, "settings": {}, "updated_ms": 0
            })
            .to_string();
            store.write(level, id, &text).unwrap();
            let reloaded = IdbPrintProfileStore::new(kv.reload());
            assert_eq!(reloaded.read(level, id).unwrap(), Some(text.as_bytes().to_vec()));
            assert_eq!(reloaded.read_last_good(level, id), Some(text));
            assert_eq!(reloaded.list(level), vec![id]);
        }
        assert!(store.list(ProfileLevel::Platform).is_empty());
    }

    #[test]
    fn print_profiles_survive_reload() {
        let kv = FakeKv::default();
        let mut store = IdbPrintProfileStore::new(kv.clone());
        let first = profile(1);
        let second = profile(2);
        store.write(ProfileLevel::User, "default", &first).unwrap();
        store.write(ProfileLevel::User, "default", &second).unwrap();
        store.keep_damaged(ProfileLevel::User, "default", b"{damaged").unwrap();
        let reloaded_kv = kv.reload();
        let reloaded = IdbPrintProfileStore::new(reloaded_kv.clone());
        assert_eq!(reloaded.read(ProfileLevel::User, "default").unwrap(), Some(second.into_bytes()));
        assert_eq!(reloaded.read_last_good(ProfileLevel::User, "default"), Some(first));
        assert_eq!(reloaded.list(ProfileLevel::User), vec!["default"]);
        assert_eq!(reloaded_kv.get(&format!("user/default/damaged/{}", sha256_hex(b"{damaged"))), Some(b"{damaged".to_vec()));
    }

    #[test]
    fn damaged_profile_is_archived_and_last_good_restored() {
        let mut kv = FakeKv::default();
        let mut store = IdbPrintProfileStore::new(kv.clone());
        let first = profile(1);
        store.write(ProfileLevel::User, "default", &first).unwrap();
        store.write(ProfileLevel::User, "default", &profile(2)).unwrap();
        kv.put_many(&[("user/default".into(), b"{".to_vec())]).unwrap();
        let reloaded_kv = kv.reload();
        let mut store = IdbPrintProfileStore::new(reloaded_kv.clone());
        let (doc, notice) = load_doc(&mut store, ProfileLevel::User, "default");
        assert_eq!(doc.unwrap().revision, 1);
        assert_eq!(reloaded_kv.get(&format!("user/default/damaged/{}", sha256_hex(b"{"))), Some(b"{".to_vec()));
        assert!(notice.unwrap().contains("restored the last good copy"));
        assert_eq!(store.read(ProfileLevel::User, "default").unwrap(), Some(b"{".to_vec()));
        assert!(store.read(ProfileLevel::Platform, "default").is_err());
        assert!(store.write(ProfileLevel::User, "../bad", &first).is_err());
    }

    #[test]
    fn profile_write_errors_leave_previous_values_intact() {
        let mut kv = FakeKv::default();
        let mut store = IdbPrintProfileStore::new(kv.clone());
        let first = profile(1);
        store.write(ProfileLevel::User, "default", &first).unwrap();
        kv.fail_writes();
        assert_eq!(store.write(ProfileLevel::User, "default", &profile(2)), Err("QuotaExceededError".into()));
        assert_eq!(store.keep_damaged(ProfileLevel::User, "default", b"broken"), Err("QuotaExceededError".into()));
        assert_eq!(store.read(ProfileLevel::User, "default").unwrap(), Some(first.into_bytes()));
    }
}

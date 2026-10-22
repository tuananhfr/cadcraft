use std::collections::BTreeMap;

use super::{MAX_PROFILE_BYTES, ProfileDoc, ProfileLevel, migrate};
use crate::hash::sha256_hex;

const MAX_STORED_PROFILES: usize = 1024;
const MAX_DAMAGED_COPIES: usize = 256;

pub trait PrintProfileStore: Send + Sync {
    fn read(&self, level: ProfileLevel, id: &str) -> Result<Option<Vec<u8>>, String>;
    fn write(&mut self, level: ProfileLevel, id: &str, json: &str) -> Result<(), String>;
    fn read_last_good(&self, level: ProfileLevel, id: &str) -> Option<String>;
    fn keep_damaged(&mut self, level: ProfileLevel, id: &str, raw: &[u8]) -> Result<(), String>;
    fn list(&self, level: ProfileLevel) -> Vec<String>;
}

#[derive(Clone, Debug, Default)]
struct StoredProfile {
    current: Option<Vec<u8>>,
    last_good: Option<String>,
    damaged: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryPrintProfileStore {
    profiles: BTreeMap<(ProfileLevel, String), StoredProfile>,
    pub fail_writes: bool,
}

pub(crate) fn validate_id(level: ProfileLevel, id: &str) -> Result<(), String> {
    if level == ProfileLevel::Platform {
        return Err("platform print settings are built into the application".into());
    }
    if level == ProfileLevel::Drawing {
        if let Some(hash) = id.strip_prefix("name:") {
            return if valid_title_hash(hash) { Ok(()) } else { Err("invalid drawing print profile id".into()) };
        }
        // Reserve the filename namespace used by browser drawings to avoid aliases on disk.
        if id.starts_with("name--") {
            return Err("reserved drawing print profile id".into());
        }
    }
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-') {
        return Err("print profile id must match [a-z0-9-]{1,64}".into());
    }
    Ok(())
}

fn valid_title_hash(hash: &str) -> bool {
    hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn decode(raw: &[u8], level: ProfileLevel, id: &str) -> Result<ProfileDoc, String> {
    validate_id(level, id)?;
    if raw.len() > MAX_PROFILE_BYTES {
        return Err("print profile exceeds 256 KiB".into());
    }
    let value = serde_json::from_slice(raw).map_err(|error| error.to_string())?;
    let doc = migrate(value).map_err(|error| error.to_string())?;
    if doc.level != level || doc.id != id {
        return Err("print profile level or id does not match its store key".into());
    }
    Ok(doc)
}

pub fn load_doc(store: &mut dyn PrintProfileStore, level: ProfileLevel, id: &str) -> (Option<ProfileDoc>, Option<String>) {
    if let Err(error) = validate_id(level, id) {
        return (None, Some(error));
    }
    let problem = match store.read(level, id) {
        Ok(None) if store.read_last_good(level, id).is_none() => return (None, None),
        Ok(None) => "current print profile is missing".into(),
        Ok(Some(raw)) => match decode(&raw, level, id) {
            Ok(doc) => return (Some(doc), None),
            Err(error) => {
                let kept = store.keep_damaged(level, id, &raw);
                match kept {
                    Ok(()) => error,
                    Err(archive_error) => format!("{error}; could not keep damaged copy: {archive_error}"),
                }
            }
        },
        Err(error) => error,
    };
    let recovered = store.read_last_good(level, id).and_then(|raw| decode(raw.as_bytes(), level, id).ok());
    let notice = if recovered.is_some() {
        format!("Print profile {level} was damaged; restored the last good copy")
    } else {
        format!("Print profile {level} was damaged; no last good copy is available")
    };
    let problem: String = problem.chars().take(256).collect();
    (recovered, Some(format!("{notice} ({problem})")))
}

pub fn drawing_key(path: Option<&str>, title: &str) -> String {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = path.filter(|path| !path.is_empty()) {
        // Oversized paths must not allocate a second input-sized buffer merely to identify a drawing.
        if path.len() > 4096 {
            return sha256_hex(path.as_bytes()).chars().take(16).collect();
        }
        let normalized = path.replace('\\', "/").to_lowercase();
        let mut parts = Vec::new();
        for part in normalized.split('/') {
            match part {
                "" | "." => {}
                ".." if parts.last().is_some_and(|part| *part != "..") => {
                    parts.pop();
                }
                _ => parts.push(part),
            }
        }
        let prefix = if normalized.starts_with("//") {
            "//"
        } else if normalized.starts_with('/') {
            "/"
        } else {
            ""
        };
        let normalized = format!("{prefix}{}", parts.join("/"));
        return sha256_hex(normalized.as_bytes()).chars().take(16).collect();
    }
    #[cfg(target_arch = "wasm32")]
    let _ = path;
    let hash: String = sha256_hex(title.as_bytes()).chars().take(16).collect();
    format!("name:{hash}")
}

impl MemoryPrintProfileStore {
    fn check_write(&self) -> Result<(), String> {
        if self.fail_writes { Err("print profile store write failed".into()) } else { Ok(()) }
    }

    fn entry(&mut self, level: ProfileLevel, id: &str) -> Result<&mut StoredProfile, String> {
        let key = (level, id.to_string());
        if !self.profiles.contains_key(&key) && self.profiles.len() >= MAX_STORED_PROFILES {
            return Err("print profile store count limit reached".into());
        }
        Ok(self.profiles.entry(key).or_default())
    }
}

impl PrintProfileStore for MemoryPrintProfileStore {
    fn read(&self, level: ProfileLevel, id: &str) -> Result<Option<Vec<u8>>, String> {
        validate_id(level, id)?;
        Ok(self.profiles.get(&(level, id.into())).and_then(|profile| profile.current.clone()))
    }

    fn write(&mut self, level: ProfileLevel, id: &str, json: &str) -> Result<(), String> {
        decode(json.as_bytes(), level, id)?;
        self.check_write()?;
        let old = self.read(level, id)?;
        if let Some(raw) = old.as_deref().filter(|raw| decode(raw, level, id).is_err()) {
            self.keep_damaged(level, id, raw)?;
        }
        let profile = self.entry(level, id)?;
        if let Some(raw) = old.filter(|raw| decode(raw, level, id).is_ok()) {
            profile.last_good = String::from_utf8(raw).ok();
        } else if profile.last_good.is_none() {
            profile.last_good = Some(json.into());
        }
        profile.current = Some(json.as_bytes().into());
        Ok(())
    }

    fn read_last_good(&self, level: ProfileLevel, id: &str) -> Option<String> {
        validate_id(level, id).ok()?;
        self.profiles.get(&(level, id.into())).and_then(|profile| profile.last_good.clone())
    }

    fn keep_damaged(&mut self, level: ProfileLevel, id: &str, raw: &[u8]) -> Result<(), String> {
        validate_id(level, id)?;
        if raw.len() > MAX_PROFILE_BYTES {
            return Err("damaged print profile exceeds 256 KiB".into());
        }
        self.check_write()?;
        let profile = self.entry(level, id)?;
        let hash = sha256_hex(raw);
        if !profile.damaged.contains_key(&hash) && profile.damaged.len() >= MAX_DAMAGED_COPIES {
            return Err("damaged print profile count limit reached".into());
        }
        profile.damaged.insert(hash, raw.into());
        Ok(())
    }

    fn list(&self, level: ProfileLevel) -> Vec<String> {
        self.profiles
            .iter()
            .filter(|((stored_level, _), profile)| *stored_level == level && profile.current.is_some())
            .map(|((_, id), _)| id.clone())
            .collect()
    }
}

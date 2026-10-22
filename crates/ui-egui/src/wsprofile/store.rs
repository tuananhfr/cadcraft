//! Where workspace profiles are kept: the `ProfileStore` each host implements and an in-memory
//! store for tests and hosts without storage.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::presets::SAFE_PRESET;
use super::schema::{SCHEMA_VERSION, WorkspaceProfile};

pub const MAX_USER_PROFILES: usize = 20;
pub const MAX_SET_BYTES: usize = 1 << 20;
pub const KEEP_SNAPSHOTS: usize = 20;
/// Snapshot revision that holds a profile set startup could not read. Revisions the app commits
/// start at 1, and pruning never removes this one, so later commits cannot erase the evidence.
pub const CORRUPT_SNAPSHOT: u64 = 0;

/// Everything committed at once, so Save As profiles and the active choice never disagree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileSet {
    #[serde(deserialize_with = "super::schema::schema_version")]
    pub schema_version: u32,
    /// A preset id or the `profile_id` of one of `profiles`.
    pub active: String,
    /// Saved user profiles, at most [`MAX_USER_PROFILES`].
    pub profiles: Vec<WorkspaceProfile>,
    /// The working profile: the user's layer over its `base_id` preset, as last committed.
    pub user: Option<WorkspaceProfile>,
    pub device: Option<super::device::DeviceState>,
    /// Fields a newer writer added, kept through a save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ProfileSet {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION, active: SAFE_PRESET.into(), profiles: Vec::new(), user: None, device: None, extra: Map::new() }
    }
}

pub trait ProfileStore {
    fn keep_backup(&mut self, _version: &str, _raw: &str) -> Result<(), String> {
        Err("this storage keeps no backup".into())
    }
    fn read_backup(&self, _version: &str) -> Option<String> {
        None
    }
    /// The [`ProfileSet`] JSON last written.
    fn read_current(&self) -> Option<String>;
    /// Replaces the current set; atomic in every implementation (old or new, never half).
    fn write_current(&mut self, json: &str) -> Result<(), String>;
    /// Keeps a copy under `revision`; keeps the newest [`KEEP_SNAPSHOTS`] plus [`CORRUPT_SNAPSHOT`].
    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String>;
    /// Stored revisions, ascending.
    fn snapshots(&self) -> Vec<u64>;
    fn read_snapshot(&self, revision: u64) -> Option<String>;
    /// Policy layers (`{"layers":[{"level":"company","patch":{..},"locks":[..]}]}`), when the host has any.
    fn read_policy(&self) -> Option<String> {
        None
    }
    /// True once after another writer changed the stored set since the last call.
    fn take_external_change(&mut self) -> bool {
        false
    }
    /// The current set as stored, also when it is too large or not UTF-8 for [`Self::read_current`].
    fn read_current_bytes(&self) -> Option<Vec<u8>> {
        self.read_current().map(String::into_bytes)
    }
    /// Keeps a copy of a set startup could not read, next to (not among) the snapshots, without
    /// replacing any copy kept before; returns a name for it. A store without this keeps the copy
    /// as snapshot [`CORRUPT_SNAPSHOT`] instead.
    fn keep_damaged(&mut self, _raw: &[u8]) -> Result<String, String> {
        Err("not supported".into())
    }
}

/// Revisions to delete so that only the newest [`KEEP_SNAPSHOTS`] remain; never [`CORRUPT_SNAPSHOT`].
pub fn expired_snapshots(revisions: &[u64]) -> Vec<u64> {
    let mut committed: Vec<u64> = revisions.iter().copied().filter(|revision| *revision != CORRUPT_SNAPSHOT).collect();
    committed.sort_unstable();
    committed.dedup();
    let excess = committed.len().saturating_sub(KEEP_SNAPSHOTS);
    committed.truncate(excess);
    committed
}

#[derive(Clone, Debug, Default)]
pub struct MemoryStore {
    pub current: Option<String>,
    pub snapshot_map: BTreeMap<u64, String>,
    pub policy: Option<String>,
    /// Makes every write fail, as a full or blocked browser storage does.
    pub fail_writes: bool,
    /// Copies kept by [`ProfileStore::keep_damaged`], by name.
    pub damaged: BTreeMap<String, Vec<u8>>,
    pub backups: BTreeMap<String, String>,
}

const WRITE_REFUSED: &str = "storage is full or blocked";

impl ProfileStore for MemoryStore {
    fn keep_backup(&mut self, version: &str, raw: &str) -> Result<(), String> {
        if self.backups.contains_key(version) {
            return Ok(());
        }
        if self.fail_writes {
            return Err(WRITE_REFUSED.into());
        }
        self.backups.insert(version.into(), raw.into());
        Ok(())
    }

    fn read_backup(&self, version: &str) -> Option<String> {
        self.backups.get(version).cloned()
    }

    fn read_current(&self) -> Option<String> {
        self.current.clone()
    }

    fn write_current(&mut self, json: &str) -> Result<(), String> {
        if self.fail_writes {
            return Err(WRITE_REFUSED.into());
        }
        self.current = Some(json.to_string());
        Ok(())
    }

    fn push_snapshot(&mut self, revision: u64, json: &str) -> Result<(), String> {
        if self.fail_writes {
            return Err(WRITE_REFUSED.into());
        }
        self.snapshot_map.insert(revision, json.to_string());
        let revisions: Vec<u64> = self.snapshot_map.keys().copied().collect();
        for expired in expired_snapshots(&revisions) {
            self.snapshot_map.remove(&expired);
        }
        Ok(())
    }

    fn snapshots(&self) -> Vec<u64> {
        self.snapshot_map.keys().copied().collect()
    }

    fn read_snapshot(&self, revision: u64) -> Option<String> {
        self.snapshot_map.get(&revision).cloned()
    }

    fn read_policy(&self) -> Option<String> {
        self.policy.clone()
    }

    fn keep_damaged(&mut self, raw: &[u8]) -> Result<String, String> {
        if self.fail_writes {
            return Err(WRITE_REFUSED.into());
        }
        if let Some((name, _)) = self.damaged.iter().find(|(_, kept)| kept.as_slice() == raw) {
            return Ok(name.clone());
        }
        let name = (1..).map(|index: u64| format!("damaged-{index}")).find(|name| !self.damaged.contains_key(name)).unwrap_or_default();
        self.damaged.insert(name.clone(), raw.to_vec());
        Ok(name)
    }
}

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
    pub schema_version: String,
    /// A preset id or the `profile_id` of one of `profiles`.
    pub active: String,
    /// Saved user profiles, at most [`MAX_USER_PROFILES`].
    pub profiles: Vec<WorkspaceProfile>,
    /// The working profile: the user's layer over its `base_id` preset, as last committed.
    pub user: Option<WorkspaceProfile>,
    /// Fields a newer writer added, kept through a save.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for ProfileSet {
    fn default() -> Self {
        Self { schema_version: SCHEMA_VERSION.into(), active: SAFE_PRESET.into(), profiles: Vec::new(), user: None, extra: Map::new() }
    }
}

pub trait ProfileStore {
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
}

const WRITE_REFUSED: &str = "storage is full or blocked";

impl ProfileStore for MemoryStore {
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
}

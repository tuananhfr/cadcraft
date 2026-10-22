//! Records entity changes across a routine's single undo step.

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};

use cadcraft_doc::{Drawing, Entity, EntityStore, Handle};
use serde::{Deserialize, Serialize};

use crate::Session;

pub const MAX_RECORDS: usize = 1_000;
pub const MAX_COMMANDS: usize = 50;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct LispAudit {
    pub at_ms: u64,
    pub source: String,
    pub sha256: Option<String>,
    pub created: u32,
    pub modified: u32,
    pub deleted: u32,
    pub commands: Vec<String>,
    pub undo_label: String,
}

/// Provenance follows the loaded bytes even if their file changes before a command runs.
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub sha256: String,
}

impl Source {
    pub(crate) fn loaded(name: &str, text: &str) -> Self {
        let path = name.replace('\\', "/");
        let name = std::path::Path::new(&path).file_name().and_then(|name| name.to_str()).unwrap_or(name).to_string();
        Self { name, sha256: super::trust::source_sha(text) }
    }
}

#[derive(Default)]
pub(super) struct Pending {
    pub source: Option<Source>,
    pub commands: Vec<String>,
}

static SINK: OnceLock<fn(&LispAudit)> = OnceLock::new();

/// Hosts may persist records; the web and CLI leave this unset.
pub fn set_audit_sink(f: fn(&LispAudit)) {
    let _ = SINK.set(f);
}

#[derive(Clone, Copy)]
enum Location<'a> {
    Model,
    Layout(&'a str),
    Block(&'a str),
}

fn stores(drawing: &Drawing) -> impl Iterator<Item = (Location<'_>, &EntityStore)> {
    std::iter::once((Location::Model, &drawing.model))
        .chain(drawing.layouts.iter().map(|layout| (Location::Layout(layout.name.as_str()), &layout.entities)))
        .chain(drawing.blocks.iter().map(|(name, block)| (Location::Block(name.as_str()), &block.entities)))
}

fn store_at<'a>(drawing: &'a Drawing, location: Location<'_>) -> Option<&'a EntityStore> {
    match location {
        Location::Model => Some(&drawing.model),
        Location::Layout(name) => drawing.layouts.iter().find(|layout| layout.name == name).map(|layout| &layout.entities),
        Location::Block(name) => drawing.blocks.get(name).map(|block| &block.entities),
    }
}

fn entity(drawing: &Drawing, handle: Handle) -> Option<&Arc<Entity>> {
    drawing.entity(handle).or_else(|| drawing.blocks.values().find_map(|block| block.entities.get(handle)))
}

pub(super) fn record(before: &Drawing, after: &Drawing, pending: Pending, undo_label: String) -> Option<LispAudit> {
    let (mut created, mut modified, mut deleted) = (0u32, 0u32, 0u32);
    // Existing store indexes avoid allocating another handle map for large drawings.
    for (location, store) in stores(after) {
        let previous = store_at(before, location);
        for current in store.iter() {
            match previous.and_then(|store| store.get(current.handle)) {
                Some(old) if old == current => {}
                Some(_) => modified = modified.saturating_add(1),
                None if entity(before, current.handle).is_some() => modified = modified.saturating_add(1),
                None => created = created.saturating_add(1),
            }
        }
    }
    for (location, store) in stores(before) {
        let next = store_at(after, location);
        for old in store.iter() {
            if !next.is_some_and(|store| store.contains(old.handle)) && entity(after, old.handle).is_none() {
                deleted = deleted.saturating_add(1);
            }
        }
    }
    if created == 0 && modified == 0 && deleted == 0 {
        return None;
    }
    let (source, sha256) = pending.source.map(|source| (source.name, Some(source.sha256))).unwrap_or_else(|| ("lisp".into(), None));
    Some(LispAudit { at_ms: crate::cmd::ubplot::now_ms(), source, sha256, created, modified, deleted, commands: pending.commands, undo_label })
}

pub(super) fn publish(ring: &mut VecDeque<LispAudit>, record: LispAudit) {
    if ring.len() == MAX_RECORDS {
        ring.pop_front();
    }
    ring.push_back(record.clone());
    if let Some(sink) = SINK.get() {
        // A host hook must not turn an already completed edit into a failed routine.
        let _ = std::panic::catch_unwind(|| sink(&record));
    }
}

impl Session {
    /// Retained records in completion order, oldest first.
    pub fn lisp_audit(&self) -> Vec<LispAudit> {
        self.lisp.audit.iter().cloned().collect()
    }

    pub(crate) fn lisp_audit_command(&mut self, id: &str) {
        if let Some(pending) = self.lisp.audit_pending.as_mut()
            && pending.commands.len() < MAX_COMMANDS
        {
            pending.commands.push(id.to_string());
        }
    }
}

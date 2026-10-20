//! Undo and redo for layout changes, kept apart from the drawing's undo (D13).

use super::schema::{WorkspaceLayout, WorkspaceProfile};

pub const HISTORY_CAP: usize = 100;

/// One undo or redo step: a layout, or a whole workspace switch with the identity it left.
// Layout steps are the common case and were this size already; boxing them would only add an allocation per step.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Layout(WorkspaceLayout),
    Switch { active: String, user: Box<WorkspaceProfile> },
}

#[derive(Clone, Debug, Default)]
pub struct LayoutHistory {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl LayoutHistory {
    /// Remembers the layout as it was before a change; a new change drops the redo branch.
    pub fn record(&mut self, before: WorkspaceLayout) {
        self.push_new(Entry::Layout(before));
    }

    /// Remembers the workspace as it was before a switch, so undo can give back its identity too.
    pub fn record_switch(&mut self, active: String, user: WorkspaceProfile) {
        self.push_new(Entry::Switch { active, user: Box::new(user) });
    }

    fn push_new(&mut self, entry: Entry) {
        if self.undo.last() != Some(&entry) {
            push_capped(&mut self.undo, entry);
        }
        self.redo.clear();
    }

    /// The step undo would take; the caller states the current state in the same kind.
    pub fn peek_undo(&self) -> Option<&Entry> {
        self.undo.last()
    }

    pub fn peek_redo(&self) -> Option<&Entry> {
        self.redo.last()
    }

    pub fn undo(&mut self, now: Entry) -> Option<Entry> {
        let previous = self.undo.pop()?;
        push_capped(&mut self.redo, now);
        Some(previous)
    }

    pub fn redo(&mut self, now: Entry) -> Option<Entry> {
        let next = self.redo.pop()?;
        push_capped(&mut self.undo, now);
        Some(next)
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

fn push_capped(stack: &mut Vec<Entry>, entry: Entry) {
    stack.push(entry);
    if stack.len() > HISTORY_CAP {
        stack.remove(0);
    }
}

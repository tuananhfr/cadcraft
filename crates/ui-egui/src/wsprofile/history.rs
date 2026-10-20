//! Undo and redo for layout changes, kept apart from the drawing's undo (D13).

use super::schema::WorkspaceLayout;

pub const HISTORY_CAP: usize = 100;

#[derive(Clone, Debug, Default)]
pub struct LayoutHistory {
    undo: Vec<WorkspaceLayout>,
    redo: Vec<WorkspaceLayout>,
}

impl LayoutHistory {
    /// Remembers the layout as it was before a change; a new change drops the redo branch.
    pub fn record(&mut self, before: WorkspaceLayout) {
        if self.undo.last() != Some(&before) {
            push_capped(&mut self.undo, before);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, now: WorkspaceLayout) -> Option<WorkspaceLayout> {
        let previous = self.undo.pop()?;
        push_capped(&mut self.redo, now);
        Some(previous)
    }

    pub fn redo(&mut self, now: WorkspaceLayout) -> Option<WorkspaceLayout> {
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

fn push_capped(stack: &mut Vec<WorkspaceLayout>, layout: WorkspaceLayout) {
    stack.push(layout);
    if stack.len() > HISTORY_CAP {
        stack.remove(0);
    }
}

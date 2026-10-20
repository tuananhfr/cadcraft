//! When a changed layout is committed: after a quiet spell, or at once when a drag, resize or
//! workspace switch ends. Saving never waits for the app to close (spec §6).

#[derive(Clone, Debug, PartialEq)]
pub struct Autosave {
    pub debounce_secs: f64,
    changed_at: Option<f64>,
    immediate: bool,
}

impl Default for Autosave {
    fn default() -> Self {
        Self { debounce_secs: 0.75, changed_at: None, immediate: false }
    }
}

impl Autosave {
    /// Restarts the wait; `now` is in seconds on any clock that only moves forward.
    pub fn changed(&mut self, now: f64) {
        self.changed_at = Some(now);
    }

    /// The pending change is saved on the next [`Self::due`] without waiting.
    pub fn interaction_ended(&mut self) {
        self.immediate = true;
    }

    /// True once per pending change, when it should be committed.
    pub fn due(&mut self, now: f64) -> bool {
        let Some(at) = self.changed_at else {
            self.immediate = false;
            return false;
        };
        // A clock that went backwards (or NaN) must not hold a save back forever.
        let waited = now - at;
        if self.immediate || !waited.is_finite() || waited < 0.0 || waited >= self.debounce_secs {
            self.clear();
            return true;
        }
        false
    }

    pub fn pending(&self) -> bool {
        self.changed_at.is_some()
    }

    pub fn clear(&mut self) {
        self.changed_at = None;
        self.immediate = false;
    }
}

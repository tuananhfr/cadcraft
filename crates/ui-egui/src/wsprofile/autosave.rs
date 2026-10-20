//! When a changed layout is committed: after a quiet spell, or at once when a drag, resize or
//! workspace switch ends. Saving never waits for the app to close (spec §6).

#[derive(Clone, Debug, PartialEq)]
pub struct Autosave {
    pub debounce_secs: f64,
    changed_at: Option<f64>,
    immediate: bool,
    /// Replaces the debounce after a failed commit, until a commit goes through.
    retry_secs: Option<f64>,
}

impl Default for Autosave {
    fn default() -> Self {
        Self { debounce_secs: 0.75, changed_at: None, immediate: false, retry_secs: None }
    }
}

impl Autosave {
    /// Restarts the wait; `now` is in seconds on any clock that only moves forward.
    pub fn changed(&mut self, now: f64) {
        self.changed_at = Some(now);
        // A new edit is saved after the usual quiet spell, even while a failed save waits to retry.
        self.retry_secs = None;
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
        if self.immediate || !waited.is_finite() || waited < 0.0 || waited >= self.wait_secs() {
            // The retry delay stays until a commit succeeds and calls `clear`.
            self.changed_at = None;
            self.immediate = false;
            return true;
        }
        false
    }

    /// Arms another attempt `delay` seconds after a commit that failed at `now`, so the change is
    /// not left only in memory.
    pub fn retry(&mut self, now: f64, delay: f64) {
        self.changed_at = Some(now);
        self.immediate = false;
        self.retry_secs = Some(delay);
    }

    /// How long a pending change waits before [`Self::due`] reports it.
    pub fn wait_secs(&self) -> f64 {
        self.retry_secs.unwrap_or(self.debounce_secs)
    }

    pub fn pending(&self) -> bool {
        self.changed_at.is_some()
    }

    /// Forgets the pending change; called once it is stored.
    pub fn clear(&mut self) {
        self.changed_at = None;
        self.immediate = false;
        self.retry_secs = None;
    }
}

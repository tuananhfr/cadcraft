//! The licence worker: every blocking account call (browser sign-in, refresh) runs here.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use erpcons_account::{Account, Error, LicenceState, Status};

use super::policy::{Denial, denial};
use super::{Notice, Opener, Shared};

/// How often a refresh asked for by `allowed()` (stale entitlements) is picked up.
const POLL: Duration = Duration::from_secs(60);
/// Renews the lease well before it runs out on a machine that stays open for days.
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 3600);
/// Syncs asked for by refused edits: at most one a minute, doubling to an hour while the
/// refusal stays (the background schedule of lease.md §6).
const RESYNC_MIN: Duration = Duration::from_secs(60);
const RESYNC_MAX: Duration = Duration::from_secs(3600);

pub(super) enum Request {
    SignIn,
    SignOut,
    ChooseOrg(String),
    /// The user pressed Retry: sync now and lift a device-cap or not-entitled block.
    Retry,
    /// Wakes the worker for a refused edit; `Shared::resync_wanted` says it is still wanted.
    Resync,
}

/// The worker's clock settings; tests shorten them.
#[derive(Clone, Copy, Debug)]
pub(super) struct Timing {
    pub poll: Duration,
    pub resync_min: Duration,
    pub resync_max: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { poll: POLL, resync_min: RESYNC_MIN, resync_max: RESYNC_MAX }
    }
}

pub(super) struct Worker {
    pub account: Arc<Account>,
    pub shared: Arc<Shared>,
    pub open_browser: Opener,
    pub wake: Box<dyn Fn() + Send>,
    pub timing: Timing,
}

pub(super) fn spawn(worker: Worker, requests: Receiver<Request>) -> std::io::Result<()> {
    std::thread::Builder::new().name("ubotcad-licence".into()).spawn(move || worker.run(&requests)).map(drop)
}

/// When the next sync for a refused edit may run.
struct Resync {
    not_before: Instant,
    failures: u32,
}

impl Resync {
    fn after(&mut self, timing: &Timing, now: Instant, result: &Result<(), Error>, still_denied: bool) {
        let delay = if still_denied || result.is_err() {
            let doubled = timing.resync_min.saturating_mul(1 << self.failures.min(6)).min(timing.resync_max);
            self.failures = self.failures.saturating_add(1);
            result.as_ref().err().and_then(Error::retry_after).map_or(doubled, |after| after.max(doubled))
        } else {
            self.failures = 0;
            timing.resync_min
        };
        self.not_before = now.checked_add(delay).unwrap_or(now);
    }

    fn wait(&self, timing: &Timing, pending: bool, now: Instant) -> Duration {
        if pending { self.not_before.saturating_duration_since(now).min(timing.poll) } else { timing.poll }
    }
}

impl Worker {
    fn run(&self, requests: &Receiver<Request>) {
        attempt("reload", self.account.reload());
        let mut resync = Resync { not_before: Instant::now(), failures: 0 };
        self.sync(&mut resync);
        self.publish();
        let mut next_refresh = later(Instant::now());
        loop {
            let pending = self.shared.resync_wanted.load(Ordering::SeqCst);
            match requests.recv_timeout(resync.wait(&self.timing, pending, Instant::now())) {
                Ok(request) => self.handle(request, &mut resync),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let now = Instant::now();
            let resync_due = now >= resync.not_before && self.shared.resync_wanted.load(Ordering::SeqCst);
            if self.account.take_refresh_request() || now >= next_refresh || resync_due {
                self.sync(&mut resync);
                next_refresh = later(now);
            }
            self.publish();
        }
    }

    fn handle(&self, request: Request, resync: &mut Resync) {
        match request {
            Request::SignIn => {
                // A second click queued behind a sign-in that has since finished.
                if !matches!(self.account.status(), Status::SignedIn(_)) {
                    self.set_notice(None);
                    let result = self.account.sign_in(|url| (self.open_browser)(url));
                    self.report_browser_flow("sign-in", result);
                }
            }
            Request::SignOut => {
                self.set_notice(None);
                attempt("sign-out", self.account.sign_out());
            }
            Request::ChooseOrg(org_id) => {
                self.set_notice(None);
                let result = self.account.select_org(Some(&org_id), |url| (self.open_browser)(url));
                self.report_browser_flow("organisation switch", result);
                self.sync(resync);
            }
            Request::Retry => {
                self.set_notice(Some(Notice::Checking));
                resync.failures = 0;
                self.sync(resync);
                let blocked = matches!(self.account.status(), Status::SignedIn(s)
                    if matches!(s.licence, LicenceState::DeviceCapReached | LicenceState::NotEntitled));
                if blocked {
                    attempt("activation retry", self.account.retry_activation());
                }
                self.clear_notice(Notice::Checking);
            }
            Request::Resync => {}
        }
    }

    /// A refresh that also answers any refused edit waiting for one.
    fn sync(&self, resync: &mut Resync) {
        // Cleared first: an edit refused while this runs asks again for the next slot.
        self.shared.resync_wanted.store(false, Ordering::SeqCst);
        let result = self.account.refresh();
        let still_denied = denial(&self.account).is_some_and(Denial::sync_may_lift);
        resync.after(&self.timing, Instant::now(), &result, still_denied);
        attempt("refresh", result);
    }

    /// Hands a due expiry warning to the UI once, then wakes it to show the new state.
    fn publish(&self) {
        if let Status::SignedIn(signed_in) = self.account.status()
            && let Some(warning) = signed_in.warning
        {
            self.set_notice(Some(Notice::Expiry { days: warning.stage.days() }));
            attempt("expiry warning", self.account.mark_warning_shown(warning.stage));
        }
        (self.wake)();
    }

    /// The user started this flow and waits on it, so a failure is shown, not only logged.
    fn report_browser_flow(&self, what: &str, result: Result<(), Error>) {
        let notice = match &result {
            Ok(()) | Err(Error::Cancelled) => None,
            Err(Error::Browser(_)) => Some(Notice::BrowserFailed),
            Err(_) => Some(Notice::SignInFailed),
        };
        attempt(what, result);
        if notice.is_some() {
            self.set_notice(notice);
        }
    }

    fn set_notice(&self, notice: Option<Notice>) {
        *self.shared.notice.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = notice;
    }

    /// Clears `notice` unless something newer replaced it.
    fn clear_notice(&self, notice: Notice) {
        let mut current = self.shared.notice.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *current == Some(notice) {
            *current = None;
        }
    }
}

fn later(now: Instant) -> Instant {
    now.checked_add(REFRESH_EVERY).unwrap_or(now)
}

/// Failures are logged only: the library keeps the last lease, the resync schedule backs off,
/// and the edit gate reports what the user can do.
fn attempt(what: &str, result: Result<(), Error>) {
    if let Err(error) = result {
        log::warn!("licence {what}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use erpcons_account::Error;

    use super::{Resync, Timing};

    #[test]
    fn resync_waits_a_minute_and_backs_off_to_an_hour_while_denied() {
        let timing = Timing::default();
        let now = Instant::now();
        let mut r = Resync { not_before: now, failures: 0 };
        let delays: Vec<u64> = (0..8)
            .map(|_| {
                r.after(&timing, now, &Ok(()), true);
                r.not_before.duration_since(now).as_secs()
            })
            .collect();
        assert_eq!(delays, [60, 120, 240, 480, 960, 1920, 3600, 3600]);
        r.after(&timing, now, &Err(Error::Timeout), false);
        assert_eq!(r.not_before.duration_since(now), Duration::from_secs(3600));
        r.after(&timing, now, &Ok(()), false);
        assert_eq!(r.not_before.duration_since(now), Duration::from_secs(60), "a lifted refusal resets the schedule");
        assert_eq!(r.wait(&timing, true, now + Duration::from_secs(45)), Duration::from_secs(15));
        assert_eq!(r.wait(&timing, false, now), timing.poll);
    }
}

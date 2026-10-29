//! The licence worker: every blocking account call (browser sign-in, refresh) runs here.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use erpcons_account::{Account, Error, Status};

use super::{Notice, Opener, Shared};

/// How often a refresh asked for by `allowed()` (stale entitlements) is picked up.
const POLL: Duration = Duration::from_secs(60);
/// Renews the lease well before it runs out on a machine that stays open for days.
const REFRESH_EVERY: Duration = Duration::from_secs(12 * 3600);

pub(super) enum Request {
    SignIn,
    SignOut,
    ChooseOrg(String),
}

pub(super) struct Worker {
    pub account: Arc<Account>,
    pub shared: Arc<Shared>,
    pub open_browser: Opener,
    pub wake: Box<dyn Fn() + Send>,
}

pub(super) fn spawn(worker: Worker, requests: Receiver<Request>) -> std::io::Result<()> {
    std::thread::Builder::new().name("ubotcad-licence".into()).spawn(move || worker.run(&requests)).map(drop)
}

impl Worker {
    fn run(&self, requests: &Receiver<Request>) {
        attempt("reload", self.account.reload());
        attempt("refresh", self.account.refresh());
        self.publish();
        let mut next_refresh = later(Instant::now());
        loop {
            match requests.recv_timeout(POLL) {
                Ok(request) => self.handle(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            let now = Instant::now();
            if self.account.take_refresh_request() || now >= next_refresh {
                attempt("refresh", self.account.refresh());
                next_refresh = later(now);
            }
            self.publish();
        }
    }

    fn handle(&self, request: Request) {
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
                attempt("refresh", self.account.refresh());
            }
        }
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
}

fn later(now: Instant) -> Instant {
    now.checked_add(REFRESH_EVERY).unwrap_or(now)
}

/// Failures are logged only: the library backs off and keeps the last lease, and the edit gate
/// reports what the user can do.
fn attempt(what: &str, result: Result<(), Error>) {
    if let Err(error) = result {
        log::warn!("licence {what}: {error}");
    }
}

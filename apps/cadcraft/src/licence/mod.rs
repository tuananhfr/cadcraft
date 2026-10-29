//! PI licence gate (feature `licensing`): drawing edits need a UbotCAD lease from PI.
//!
//! The account's blocking calls run on a worker thread; the edit policy and the Account dialog
//! read only its cached state, so the UI never waits for the network.

mod config;
mod host;
mod policy;
mod worker;

use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

use cadcraft_engine::EditPolicy;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::account::AccountHost;
use erpcons_account::{Account, Config};

use config::pi_config;
use policy::LeasePolicy;

type Opener = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

/// What the worker tells the UI besides the account state.
#[derive(Default)]
struct Shared {
    notice: Mutex<Option<Notice>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Expiry { days: u32 },
}

pub struct LicenceService {
    account: Arc<Account>,
    requests: Sender<worker::Request>,
    shared: Arc<Shared>,
}

impl LicenceService {
    /// Opens the account with the OS credential store and starts the worker; no network call
    /// happens before it returns.
    pub fn start(config: Config, wake: impl Fn() + Send + 'static) -> Result<Self, erpcons_account::Error> {
        let account = Arc::new(Account::with_os_store(config)?);
        Ok(Self::start_with(account, Arc::new(|url: &str| open::that(url).map_err(|e| e.to_string())), wake))
    }

    pub(crate) fn start_with(account: Arc<Account>, open_browser: Opener, wake: impl Fn() + Send + 'static) -> Self {
        let (requests, received) = channel();
        let shared = Arc::new(Shared::default());
        let worker = worker::Worker { account: account.clone(), shared: shared.clone(), open_browser, wake: Box::new(wake) };
        if let Err(error) = worker::spawn(worker, received) {
            // The cached lease still decides; only sign-in and renewal are unavailable.
            log::error!("licence worker did not start: {error}");
        }
        Self { account, requests, shared }
    }

    pub fn policy(&self) -> Box<dyn EditPolicy> {
        Box::new(LeasePolicy(self.account.clone()))
    }

    pub fn host(&self) -> Rc<dyn AccountHost> {
        Rc::new(host::Host { account: self.account.clone(), requests: self.requests.clone(), shared: self.shared.clone() })
    }
}

/// Refuses every edit when the account could not be opened at all.
struct Unavailable;

impl EditPolicy for Unavailable {
    fn check(&self) -> Result<(), String> {
        Err(policy::SIGN_IN.to_owned())
    }
}

/// Gates `app`'s drawing edits on the PI lease and gives it the Account dialog.
pub fn install(app: &mut CadApp, wake: impl Fn() + Send + 'static) {
    let Some(config) = pi_config(env!("CARGO_PKG_VERSION")) else {
        // Only debug builds get here (releases must name PI_ISSUER); they stay ungated.
        log::warn!("licensing: no PI issuer (set UBOTCAD_PI_ISSUER); drawing edits are not gated");
        return;
    };
    match LicenceService::start(config, wake) {
        Ok(service) => {
            app.session.set_edit_policy(service.policy());
            app.services.account = Some(service.host());
        }
        Err(error) => {
            // Failing open would let anyone unlock editing by breaking the PI folder.
            log::error!("licensing: PI account unavailable: {error}");
            app.session.set_edit_policy(Box::new(Unavailable));
        }
    }
}

// Plain `cfg(test)` (this module already needs `licensing`) so clippy treats it as test code.
#[cfg(test)]
mod tests;

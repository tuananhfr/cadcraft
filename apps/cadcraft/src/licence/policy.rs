//! The edit gate: drawing edits need `ubotbuild.cad-use` in an active lease of this device.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;

use cadcraft_engine::EditPolicy;
use erpcons_account::{Account, LicenceState, Status};

use super::Shared;
use super::worker::Request;

/// Also the form the lease lists it in (`LicenceState::Active::features`).
pub const CAD_USE: &str = "ubotbuild.cad-use";

// English i18n keys: the command line and status bar look engine messages up in the catalogs
// (`ui-egui/src/i18n/vi.tsv`) when they display them.
pub const SIGN_IN: &str = "Sign in to PI ID to use the Pro edition";
pub const CHOOSE_ORG: &str = "Choose your organisation in PI ID";
pub const REVOKED: &str = "This device was removed from your PI ID account. Sign in again to use the Pro edition";
pub const DEVICE_CAP: &str = "Device limit reached for your PI ID plan. Remove a device in PI ID, then press Retry in Help > Account";
pub const NOT_ACTIVATED: &str = "This device is not activated for the Pro edition yet";
pub const RENEW_ONLINE: &str = "Connect to the internet to renew";
pub const RENEW_FAILED: &str = "Could not renew your UbotCAD licence; UbotCAD keeps retrying";
pub const NO_PLAN: &str = "This account has no UbotCAD desktop plan";
pub const UNAVAILABLE: &str = "PI ID licensing is unavailable on this machine; edits are disabled";

/// Why edits are refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    SignIn,
    ChooseOrg,
    Revoked,
    DeviceCap,
    NotActivated,
    /// No OS credential store: the device key cannot be kept, so there is no lease either.
    SessionOnly,
    RenewOffline,
    RenewFailed,
    NoPlan,
}

impl Denial {
    pub fn reason(self) -> &'static str {
        match self {
            Denial::SignIn => SIGN_IN,
            Denial::ChooseOrg => CHOOSE_ORG,
            Denial::Revoked => REVOKED,
            Denial::DeviceCap => DEVICE_CAP,
            Denial::NotActivated | Denial::SessionOnly => NOT_ACTIVATED,
            Denial::RenewOffline => RENEW_ONLINE,
            Denial::RenewFailed => RENEW_FAILED,
            Denial::NoPlan => NO_PLAN,
        }
    }

    /// A plain sync with PI may lift it: activation retried, lease renewed, a plan bought or an
    /// organisation joined since the last sync.
    pub fn sync_may_lift(self) -> bool {
        matches!(self, Denial::ChooseOrg | Denial::NotActivated | Denial::RenewOffline | Denial::RenewFailed | Denial::NoPlan)
    }

    /// The Account dialog offers Retry; a device-cap block lifts only on the user's retry
    /// (lease.md §6).
    pub fn retry_offered(self) -> bool {
        self.sync_may_lift() || self == Denial::DeviceCap
    }
}

/// Reads only the account's cached state, so it is cheap enough to run before every edit.
pub struct LeasePolicy {
    pub account: Arc<Account>,
    pub shared: Arc<Shared>,
    pub requests: Sender<Request>,
}

impl EditPolicy for LeasePolicy {
    fn check(&self) -> Result<(), String> {
        let Some(denial) = denial(&self.account) else { return Ok(()) };
        // The worker rate-limits syncs; only the first refusal sends, so refusals never pile up
        // in the queue.
        if denial.sync_may_lift() && !self.shared.resync_wanted.swap(true, Ordering::SeqCst) {
            let _ = self.requests.send(Request::Resync);
        }
        Err(denial.reason().to_owned())
    }
}

/// Why edits are refused now, `None` when they are allowed.
pub fn denial(account: &Account) -> Option<Denial> {
    denial_in(&account.status(), |feature| account.allowed(feature))
}

/// [`denial`] of a status snapshot; `allowed` is asked only while the lease lists the feature.
pub fn denial_in(status: &Status, allowed: impl FnOnce(&str) -> bool) -> Option<Denial> {
    let signed_in = match status {
        Status::Guest | Status::SigningIn => return Some(Denial::SignIn),
        Status::SignedIn(signed_in) => signed_in,
    };
    // PI licenses through organisations only (D418); each user gets a one-person organisation.
    if signed_in.active_org.is_none() {
        return Some(Denial::ChooseOrg);
    }
    // `allowed()` alone also grants on fresh online entitlements, which the worker keeps fresh;
    // only this device's lease reflects a revoked or capped device, so it must grant too.
    match &signed_in.licence {
        LicenceState::DeviceRevoked => Some(Denial::Revoked),
        LicenceState::DeviceCapReached => Some(Denial::DeviceCap),
        LicenceState::NotEntitled => Some(Denial::NoPlan),
        LicenceState::NotActivated => Some(Denial::NotActivated),
        LicenceState::SessionOnly => Some(Denial::SessionOnly),
        LicenceState::Inactive { .. } if !signed_in.online => Some(Denial::RenewOffline),
        // Online yet expired: renewal failed or has not run yet, which is not "no plan".
        LicenceState::Inactive { .. } => Some(Denial::RenewFailed),
        LicenceState::Active { features, .. } if features.iter().any(|f| f == CAD_USE) && allowed(CAD_USE) => None,
        LicenceState::Active { .. } => Some(Denial::NoPlan),
    }
}

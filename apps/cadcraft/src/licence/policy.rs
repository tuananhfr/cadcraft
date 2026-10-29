//! The edit gate: drawing edits need `ubotbuild.cad.use` in an active lease of this device.

use std::sync::Arc;

use cadcraft_engine::EditPolicy;
use erpcons_account::{Account, LicenceState, Status};

/// Also the form the lease lists it in (`LicenceState::Active::features`).
pub const CAD_USE: &str = "ubotbuild.cad.use";

// English i18n keys: the command line and status bar look engine messages up in the catalogs
// (`ui-egui/src/i18n/vi.tsv`) when they display them.
pub const SIGN_IN: &str = "Sign in to PI ID to use the Pro edition";
pub const CHOOSE_ORG: &str = "Choose your organisation in PI ID";
pub const REVOKED: &str = "This device was removed from your PI ID account. Sign in again to use the Pro edition";
pub const DEVICE_CAP: &str = "Device limit reached for your PI ID plan. Remove another device in PI ID, then retry";
pub const NOT_ACTIVATED: &str = "This device is not activated for the Pro edition yet";
pub const RENEW_ONLINE: &str = "Connect to the internet to renew";
pub const NO_PLAN: &str = "This account has no UbotCAD desktop plan";
pub const UNAVAILABLE: &str = "PI ID licensing is unavailable on this machine; edits are disabled";

/// Reads only the account's cached state, so it is cheap enough to run before every edit.
pub struct LeasePolicy(pub Arc<Account>);

impl EditPolicy for LeasePolicy {
    fn check(&self) -> Result<(), String> {
        match denial(&self.0) {
            Some(reason) => Err(reason.to_owned()),
            None => Ok(()),
        }
    }
}

/// Why edits are refused now, `None` when they are allowed.
pub fn denial(account: &Account) -> Option<&'static str> {
    let signed_in = match account.status() {
        Status::Guest | Status::SigningIn => return Some(SIGN_IN),
        Status::SignedIn(signed_in) => signed_in,
    };
    // PI licenses through organisations only (D418); each user gets a one-person organisation.
    if signed_in.active_org.is_none() {
        return Some(CHOOSE_ORG);
    }
    // `allowed()` alone also grants on fresh online entitlements, which the worker keeps fresh;
    // only this device's lease reflects a revoked or capped device, so it must grant too.
    match &signed_in.licence {
        LicenceState::DeviceRevoked => Some(REVOKED),
        LicenceState::DeviceCapReached => Some(DEVICE_CAP),
        LicenceState::NotActivated | LicenceState::SessionOnly => Some(NOT_ACTIVATED),
        LicenceState::Inactive { .. } if !signed_in.online => Some(RENEW_ONLINE),
        LicenceState::Inactive { .. } => Some(NO_PLAN),
        LicenceState::Active { features, .. } if features.iter().any(|f| f == CAD_USE) && account.allowed(CAD_USE) => None,
        LicenceState::Active { .. } => Some(NO_PLAN),
    }
}

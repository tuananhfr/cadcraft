//! The edit gate: drawing edits need `ubotbuild.cad.use` from a lease or fresh entitlements.

use std::sync::Arc;

use cadcraft_engine::EditPolicy;
use erpcons_account::{Account, LicenceState, Status};

pub const CAD_USE: &str = "ubotbuild.cad.use";

// English i18n keys: the command line and status bar look engine messages up in the catalogs
// (`ui-egui/src/i18n/vi.tsv`) when they display them.
pub const SIGN_IN: &str = "Sign in to PI ID to use the Pro edition";
pub const CHOOSE_ORG: &str = "Choose your organisation in PI ID";
pub const RENEW_ONLINE: &str = "Connect to the internet to renew";
pub const NO_PLAN: &str = "This account has no UbotCAD desktop plan";

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
    if account.allowed(CAD_USE) {
        return None;
    }
    if matches!(signed_in.licence, LicenceState::Inactive { .. }) && !signed_in.online {
        return Some(RENEW_ONLINE);
    }
    Some(NO_PLAN)
}

//! The Account dialog's view of the service; runs on the UI thread every frame.

use std::sync::mpsc::Sender;
use std::sync::{Arc, PoisonError};

use cadcraft_ui_egui::account::{AccountHost, AccountSummary, OrgChoice};
use cadcraft_ui_egui::i18n;
use erpcons_account::{Account, LicenceState, MembershipStatus, Status};

use super::policy::CAD_USE;
use super::worker::Request;
use super::{Notice, Shared};

pub(super) struct Host {
    pub account: Arc<Account>,
    pub requests: Sender<Request>,
    pub shared: Arc<Shared>,
}

impl Host {
    fn send(&self, request: Request) {
        if self.requests.send(request).is_err() {
            log::warn!("licence worker is not running; the request was dropped");
        }
    }
}

impl AccountHost for Host {
    fn summary(&self) -> AccountSummary {
        let notice = *self.shared.notice.lock().unwrap_or_else(PoisonError::into_inner);
        summary(self.account.status(), notice)
    }

    fn sign_in(&self) {
        if !matches!(self.account.status(), Status::SigningIn) {
            self.send(Request::SignIn);
        }
    }

    fn sign_out(&self) {
        // A browser sign-in can wait for minutes; sign-out must not queue behind it.
        self.account.cancel_sign_in();
        self.send(Request::SignOut);
    }

    fn choose_org(&self, org_id: &str) {
        self.send(Request::ChooseOrg(org_id.to_owned()));
    }
}

/// Built from cached state only. The plan reads the lease rather than `allowed()`, which would
/// ask the worker for a refresh every minute while the dialog polls it.
fn summary(status: Status, notice: Option<Notice>) -> AccountSummary {
    let view_only = i18n::t("View only").to_owned();
    let Status::SignedIn(signed_in) = status else {
        return AccountSummary { plan: view_only, notice: notice.map(Notice::text), ..AccountSummary::default() };
    };
    let orgs: Vec<OrgChoice> = signed_in
        .orgs
        .iter()
        .filter(|m| m.status == MembershipStatus::Active)
        .map(|m| OrgChoice { id: m.org_id.clone(), name: m.name.clone() })
        .collect();
    let org = signed_in.active_org.as_ref().map(|id| signed_in.orgs.iter().find(|m| &m.org_id == id).map_or_else(|| id.clone(), |m| m.name.clone()));
    let pro_until = match &signed_in.licence {
        LicenceState::Active { features, expires_at, .. } if signed_in.active_org.is_some() && features.iter().any(|f| f == CAD_USE) => {
            Some(*expires_at)
        }
        _ => None,
    };
    let no_org = signed_in.user.is_some() && orgs.is_empty();
    AccountSummary {
        signed_in: true,
        user: signed_in.user.map(|u| if u.display_name.is_empty() { u.email.unwrap_or_default() } else { u.display_name }),
        plan: if pro_until.is_some() { "Pro".to_owned() } else { view_only },
        offline_until: pro_until.and_then(utc_date),
        notice: notice.map(Notice::text).or_else(|| no_org.then(|| i18n::t(NO_ORG).to_owned())),
        org,
        active_org_id: signed_in.active_org,
        orgs,
    }
}

const NO_ORG: &str = "This PI ID account does not belong to any organisation";
pub(super) const SIGN_IN_FAILED: &str = "Could not complete PI ID sign-in. Try again";
pub(super) const BROWSER_FAILED: &str = "Could not open the web browser";

impl Notice {
    fn text(self) -> String {
        match self {
            Notice::Expiry { days } => i18n::format("Your UbotCAD licence expires in {days} days", &[("days", days.to_string())]),
            Notice::SignInFailed => i18n::t(SIGN_IN_FAILED).to_owned(),
            Notice::BrowserFailed => i18n::t(BROWSER_FAILED).to_owned(),
        }
    }
}

/// `YYYY-MM-DD` of a Unix time in UTC (days-to-civil, proleptic Gregorian).
fn utc_date(unix: i64) -> Option<String> {
    let z = unix.div_euclid(86_400).checked_add(719_468)?;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era.checked_mul(400)?.checked_add(yoe)?.checked_add(i64::from(month <= 2))?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use erpcons_account::{LicenceState, Membership, MembershipStatus, SignedIn, Status, User};

    use super::{NO_ORG, summary, utc_date};

    fn signed_in(orgs: Vec<Membership>, active_org: Option<&str>) -> Status {
        Status::SignedIn(Box::new(SignedIn {
            user: Some(User { user_id: "u".into(), display_name: "An".into(), email: None }),
            orgs,
            active_org: active_org.map(str::to_owned),
            licence: LicenceState::NotActivated,
            licences: Default::default(),
            warning: None,
            online: true,
            session_only: false,
            reauth_required: false,
        }))
    }

    fn member(id: &str, status: MembershipStatus) -> Membership {
        Membership { org_id: id.into(), name: format!("Org {id}"), role: "member".into(), status }
    }

    #[test]
    fn summary_offers_active_orgs_and_flags_users_without_one() {
        let orgs = vec![member("a", MembershipStatus::Active), member("b", MembershipStatus::Suspended)];
        let s = summary(signed_in(orgs, Some("b")), None);
        assert_eq!(s.orgs.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(s.org.as_deref(), Some("Org b"));
        assert_eq!(s.active_org_id.as_deref(), Some("b"));
        assert_eq!(s.notice, None);
        let s = summary(signed_in(vec![member("b", MembershipStatus::Suspended)], None), None);
        assert!(s.orgs.is_empty());
        assert_eq!(s.notice.as_deref(), Some(NO_ORG));
        assert_eq!(summary(Status::Guest, None).notice, None);
    }

    #[test]
    fn utc_dates() {
        assert_eq!(utc_date(0).as_deref(), Some("1970-01-01"));
        assert_eq!(utc_date(951_782_400).as_deref(), Some("2000-02-29"));
        assert_eq!(utc_date(1_792_828_800).as_deref(), Some("2026-10-24"));
        assert_eq!(utc_date(-1).as_deref(), Some("1969-12-31"));
        assert!(utc_date(i64::MAX).is_some() && utc_date(i64::MIN).is_some());
    }
}

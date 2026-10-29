//! The Account dialog's view of the service; runs on the UI thread every frame.

use std::sync::mpsc::Sender;
use std::sync::{Arc, PoisonError};

use cadcraft_ui_egui::account::{AccountHost, AccountSummary, OrgChoice};
use cadcraft_ui_egui::i18n;
use erpcons_account::{Account, LicenceState, MembershipStatus, Status};

use super::policy::{CAD_USE, denial_in};
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
        summary(self.account.status(), notice, |feature| self.account.allowed(feature))
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

    fn retry(&self) {
        self.send(Request::Retry);
    }
}

/// Built from cached state only. The plan reads the lease rather than `allowed()`, which would
/// ask the worker for a refresh every minute while the dialog polls it; the refusal asks
/// `allowed` only while the lease lists the feature, where it answers from the lease.
fn summary(status: Status, notice: Option<Notice>, allowed: impl FnOnce(&str) -> bool) -> AccountSummary {
    let view_only = i18n::t("View only").to_owned();
    let denial = denial_in(&status, allowed);
    let refused = denial.map(|d| i18n::t(d.reason()).to_owned());
    let can_retry = denial.is_some_and(|d| d.retry_offered());
    let Status::SignedIn(signed_in) = status else {
        return AccountSummary { plan: view_only, notice: notice.map(Notice::text), refused, ..AccountSummary::default() };
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
        refused,
        can_retry,
    }
}

const NO_ORG: &str = "This PI ID account does not belong to any organisation";
pub(super) const SIGN_IN_FAILED: &str = "Could not complete PI ID sign-in. Try again";
pub(super) const BROWSER_FAILED: &str = "Could not open the web browser";
pub(super) const CHECKING: &str = "Checking your UbotCAD licence with PI ID…";
const EXPIRES_IN_A_DAY: &str = "Your UbotCAD licence expires in 1 day";

impl Notice {
    fn text(self) -> String {
        match self {
            Notice::Expiry { days: 1 } => i18n::t(EXPIRES_IN_A_DAY).to_owned(),
            Notice::Expiry { days } => i18n::format("Your UbotCAD licence expires in {days} days", &[("days", days.to_string())]),
            Notice::SignInFailed => i18n::t(SIGN_IN_FAILED).to_owned(),
            Notice::BrowserFailed => i18n::t(BROWSER_FAILED).to_owned(),
            Notice::Checking => i18n::t(CHECKING).to_owned(),
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
    use crate::licence::Notice;

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
        let s = summary(signed_in(orgs, Some("b")), None, |_| true);
        assert_eq!(s.orgs.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(s.org.as_deref(), Some("Org b"));
        assert_eq!(s.active_org_id.as_deref(), Some("b"));
        assert_eq!(s.notice, None);
        let s = summary(signed_in(vec![member("b", MembershipStatus::Suspended)], None), None, |_| true);
        assert!(s.orgs.is_empty());
        assert_eq!(s.notice.as_deref(), Some(NO_ORG));
        assert_eq!(summary(Status::Guest, None, |_| true).notice, None);
    }

    #[test]
    fn summary_gives_the_refusal_and_offers_retry_where_it_can_help() {
        use crate::licence::policy::{CHOOSE_ORG, DEVICE_CAP, NOT_ACTIVATED, SIGN_IN};
        let guest = summary(Status::Guest, None, |_| true);
        assert_eq!((guest.refused.as_deref(), guest.can_retry), (Some(SIGN_IN), false));
        let no_org = summary(signed_in(vec![member("a", MembershipStatus::Active)], None), None, |_| true);
        assert_eq!((no_org.refused.as_deref(), no_org.can_retry), (Some(CHOOSE_ORG), true));
        let not_activated = summary(signed_in(vec![member("a", MembershipStatus::Active)], Some("a")), None, |_| true);
        assert_eq!((not_activated.refused.as_deref(), not_activated.can_retry), (Some(NOT_ACTIVATED), true));
        let mut capped = signed_in(vec![member("a", MembershipStatus::Active)], Some("a"));
        if let Status::SignedIn(s) = &mut capped {
            s.licence = LicenceState::DeviceCapReached;
        }
        let capped = summary(capped, Some(Notice::Checking), |_| true);
        assert_eq!((capped.refused.as_deref(), capped.can_retry), (Some(DEVICE_CAP), true));
        assert_eq!(capped.notice.as_deref(), Some(super::CHECKING));
    }

    #[test]
    fn expiry_notice_reads_one_day_in_the_singular() {
        assert_eq!(Notice::Expiry { days: 1 }.text(), super::EXPIRES_IN_A_DAY);
        assert_eq!(Notice::Expiry { days: 3 }.text(), "Your UbotCAD licence expires in 3 days");
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

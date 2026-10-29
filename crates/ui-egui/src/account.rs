//! Account dialog for hosts that gate the app behind a sign-in; the host implements [`AccountHost`].

use std::rc::Rc;

use serde_json::{Value, json};

use crate::CadApp;

pub const DIALOG: &str = "account";
pub const COMMAND: &str = "ui.account";

/// What the dialog shows; the host builds it from cached state and must not block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountSummary {
    pub signed_in: bool,
    pub user: Option<String>,
    pub plan: String,
    pub offline_until: Option<String>,
    pub notice: Option<String>,
    /// Active organisation's display name; `None` is the personal context.
    pub org: Option<String>,
    /// Active organisation's id, to mark it among `orgs` (names need not be unique).
    pub active_org_id: Option<String>,
    /// Organisations the user may choose; PI licenses through an organisation only.
    pub orgs: Vec<OrgChoice>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrgChoice {
    pub id: String,
    pub name: String,
}

pub trait AccountHost {
    fn summary(&self) -> AccountSummary;
    fn sign_in(&self);
    fn sign_out(&self);
    /// Enter organisation `org_id`; hosts without organisations keep the default.
    fn choose_org(&self, _org_id: &str) {}
}

#[derive(Default)]
pub struct State {
    /// Last notice posted to the status line, so an unchanged one is not posted every frame.
    posted_notice: Option<String>,
}

pub fn enabled(app: &CadApp) -> bool {
    app.services.account.is_some()
}

pub fn route(app: &mut CadApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    if id != COMMAND {
        return None;
    }
    if !enabled(app) {
        return Some(Err("no account service in this build".into()));
    }
    app.ui.dialog = Some(DIALOG.to_string());
    Some(Ok(json!(null)))
}

/// Forward the host's notice to the status line once per distinct content.
pub fn poll(app: &mut CadApp) {
    let Some(host) = app.services.account.clone() else { return };
    let notice = host.summary().notice;
    if notice == app.account.posted_notice {
        return;
    }
    if let Some(text) = &notice {
        app.set_status(text.clone());
    }
    app.account.posted_notice = notice;
}

pub fn choose_org(host: &Rc<dyn AccountHost>, org_id: &str) {
    host.choose_org(org_id);
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let Some(host) = app.services.account.clone() else {
        *open = false;
        return;
    };
    let summary = host.summary();
    egui::Window::new(crate::tl!("Account")).id(egui::Id::new(DIALOG)).open(open).resizable(false).show(ctx, |ui| {
        match (&summary.user, summary.signed_in) {
            (Some(user), true) => ui.label(crate::tf!("Signed in as {user}", user = user)),
            _ => ui.label(crate::tl!("Not signed in")),
        };
        if let Some(org) = &summary.org {
            ui.label(crate::tf!("Organisation: {org}", org = org));
        }
        if summary.signed_in && !summary.orgs.is_empty() {
            ui.label(crate::tl!("Choose organisation"));
            for choice in &summary.orgs {
                let active = summary.active_org_id.as_deref() == Some(choice.id.as_str());
                if ui.selectable_label(active, choice.name.as_str()).clicked() && !active {
                    choose_org(&host, &choice.id);
                }
            }
        }
        ui.label(crate::tf!("Plan: {plan}", plan = summary.plan));
        if let Some(until) = &summary.offline_until {
            ui.label(crate::tf!("Offline until {date}", date = until));
        }
        ui.separator();
        if summary.signed_in {
            if ui.button(crate::tl!("Sign out")).clicked() {
                host.sign_out();
            }
        } else if ui.button(crate::tl!("Sign in to PI ID")).clicked() {
            host.sign_in();
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use cadcraft_engine::Session;
    use serde_json::Value;

    use super::{AccountHost, AccountSummary};
    use crate::menus::Entry;
    use crate::{CadApp, Services};

    #[derive(Default)]
    struct Fake {
        summary: RefCell<AccountSummary>,
        chosen: RefCell<Vec<String>>,
    }

    impl AccountHost for Fake {
        fn summary(&self) -> AccountSummary {
            self.summary.borrow().clone()
        }
        fn sign_in(&self) {}
        fn sign_out(&self) {}
        fn choose_org(&self, org_id: &str) {
            self.chosen.borrow_mut().push(org_id.to_string());
        }
    }

    fn app_with(fake: &Rc<Fake>) -> CadApp {
        let host: Rc<dyn AccountHost> = fake.clone();
        CadApp::new(Session::empty(), Services { account: Some(host), ..Services::default() })
    }

    fn account_enabled(app: &CadApp) -> Option<bool> {
        let tree = crate::menus::tree(app);
        let (_, help) = tree.iter().find(|(m, _)| m == "Help")?;
        help.iter().find_map(|e| match e {
            Entry::Item { id, enabled, .. } if id == super::COMMAND => Some(*enabled),
            _ => None,
        })
    }

    #[test]
    fn account_command_hidden_without_host() {
        let mut app = CadApp::new(Session::empty(), Services::default());
        assert_eq!(account_enabled(&app), Some(false));
        assert!(app.run("ui.account", Value::Null).is_err());
        assert_eq!(app.ui.dialog, None);
    }

    #[test]
    fn account_command_opens_dialog() {
        let fake = Rc::new(Fake::default());
        let mut app = app_with(&fake);
        assert_eq!(account_enabled(&app), Some(true));
        app.run("ui.account", Value::Null).unwrap();
        assert_eq!(app.ui.dialog.as_deref(), Some("account"));
    }

    #[test]
    fn account_notice_goes_to_status() {
        let fake = Rc::new(Fake::default());
        fake.summary.borrow_mut().notice = Some("Licence expired".into());
        let mut app = app_with(&fake);
        super::poll(&mut app);
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Licence expired"));

        app.status = None;
        super::poll(&mut app);
        assert!(app.status.is_none(), "an unchanged notice is not posted again");

        fake.summary.borrow_mut().notice = Some("Signed out".into());
        super::poll(&mut app);
        assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Signed out"));
    }

    #[test]
    fn account_choose_org_calls_host() {
        let fake = Rc::new(Fake::default());
        fake.summary.borrow_mut().orgs = vec![super::OrgChoice { id: "org-a".into(), name: "Acme".into() }];
        let app = app_with(&fake);
        let host = app.services.account.clone().unwrap();
        super::choose_org(&host, "org-a");
        assert_eq!(*fake.chosen.borrow(), ["org-a"]);
    }
}

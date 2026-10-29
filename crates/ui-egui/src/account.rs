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
}

pub trait AccountHost {
    fn summary(&self) -> AccountSummary;
    fn sign_in(&self);
    fn sign_out(&self);
    /// The licence is personal-only; hosts that can leave an organisation context override this.
    fn switch_to_personal(&self) {}
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

pub fn switch_to_personal(host: &Rc<dyn AccountHost>) {
    host.switch_to_personal();
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
            if ui.button(crate::tl!("Switch to personal account")).clicked() {
                switch_to_personal(&host);
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
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use cadcraft_engine::Session;
    use serde_json::Value;

    use super::{AccountHost, AccountSummary};
    use crate::menus::Entry;
    use crate::{CadApp, Services};

    #[derive(Default)]
    struct Fake {
        summary: RefCell<AccountSummary>,
        switched: Cell<u32>,
    }

    impl AccountHost for Fake {
        fn summary(&self) -> AccountSummary {
            self.summary.borrow().clone()
        }
        fn sign_in(&self) {}
        fn sign_out(&self) {}
        fn switch_to_personal(&self) {
            self.switched.set(self.switched.get() + 1);
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
    fn account_switch_to_personal_calls_host() {
        let fake = Rc::new(Fake::default());
        fake.summary.borrow_mut().org = Some("Acme".into());
        let app = app_with(&fake);
        let host = app.services.account.clone().unwrap();
        super::switch_to_personal(&host);
        assert_eq!(fake.switched.get(), 1);
    }
}

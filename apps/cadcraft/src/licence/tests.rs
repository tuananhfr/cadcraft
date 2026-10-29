use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use erpcons_account::{Account, Clock, Config, LicenceState, MemorySecretStore, PinnedKey, Product, Status};
use pi_mock::{DEFAULT_USER, Entitlement, Fault, ORG_A, ORG_B, Scenario, Server};

use super::LicenceService;
use super::host::{BROWSER_FAILED, SIGN_IN_FAILED};
use super::policy::{CHOOSE_ORG, DEVICE_CAP, NO_PLAN, NOT_ACTIVATED, REVOKED, SIGN_IN};

const T0: i64 = 1_792_828_800;
const USE: &str = "ubotbuild.cad.use";

struct TestClock {
    unix: AtomicI64,
    mono_ms: AtomicU64,
}

impl Clock for TestClock {
    fn unix_now(&self) -> i64 {
        self.unix.load(Ordering::SeqCst)
    }
    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.mono_ms.load(Ordering::SeqCst))
    }
}

struct Env {
    server: Server,
    dir: tempfile::TempDir,
    store: Arc<MemorySecretStore>,
    clock: Arc<TestClock>,
}

impl Env {
    fn new(mut scenario: Scenario) -> Self {
        scenario.now.get_or_insert(T0);
        Self {
            server: Server::start(scenario),
            dir: tempfile::tempdir().unwrap(),
            store: Arc::new(MemorySecretStore::new()),
            clock: Arc::new(TestClock { unix: AtomicI64::new(T0), mono_ms: AtomicU64::new(1_000_000) }),
        }
    }

    fn config(&self) -> Config {
        let mut c = super::config::config_for(self.server.issuer(), "0.9.0");
        c.platform_keys = vec![PinnedKey { kid: self.server.lease_key_id(), key: self.server.lease_public_key() }];
        c.state_dir = Some(self.dir.path().to_path_buf());
        c.clock = self.clock.clone();
        c.timeout = Duration::from_secs(2);
        c.max_retry_wait = Duration::from_secs(3);
        c.sign_in_timeout = Duration::from_secs(20);
        c
    }

    fn account(&self) -> Arc<Account> {
        Arc::new(Account::new(self.config(), self.store.clone()).unwrap())
    }

    /// A service as `main` starts it, but with the test browser and an in-memory credential store.
    fn service(&self) -> (LicenceService, Arc<Account>) {
        self.service_with(Arc::new(browser))
    }

    fn service_with(&self, open_browser: super::Opener) -> (LicenceService, Arc<Account>) {
        let account = self.account();
        (LicenceService::start_with(account.clone(), open_browser, || {}), account)
    }

    /// Moves client wall clock, client monotonic clock and platform time together.
    fn advance(&self, secs: i64) {
        self.clock.unix.fetch_add(secs, Ordering::SeqCst);
        self.clock.mono_ms.fetch_add(secs.unsigned_abs() * 1000, Ordering::SeqCst);
        let now = self.clock.unix_now();
        self.server.update_scenario(|s| s.now = Some(now));
    }
}

/// The "system browser": follows the authorization redirect to the loopback listener.
fn browser(url: &str) -> Result<(), String> {
    let url = url.to_owned();
    std::thread::spawn(move || {
        let _ = ureq::get(&url).call();
    });
    Ok(())
}

fn wait_for(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn licence(account: &Account) -> Option<LicenceState> {
    match account.status() {
        Status::SignedIn(s) => Some(s.licence),
        _ => None,
    }
}

fn in_org(account: &Account, org: &str) -> bool {
    matches!(account.status(), Status::SignedIn(s) if s.active_org.as_deref() == Some(org))
}

/// `ubotbuild.cad.use` granted to the default user in organisation A only.
fn org_scenario() -> Scenario {
    Scenario { entitlements: vec![Entitlement::new("ent_cad", DEFAULT_USER, Some(ORG_A), USE)], ..Scenario::default() }
}

/// What an earlier run of the app left behind: signed in, in organisation A, with its lease.
fn prepare_org_session(env: &Env) {
    let account = env.account();
    account.sign_in(browser).unwrap();
    account.select_org(Some(ORG_A), browser).unwrap();
}

#[test]
fn guest_is_denied_with_sign_in_reason() {
    let env = Env::new(org_scenario());
    let (service, _) = env.service();
    assert_eq!(service.policy().check(), Err(SIGN_IN.to_string()));
    assert!(!service.host().summary().signed_in);
}

#[test]
fn entitled_user_can_edit() {
    let env = Env::new(org_scenario());
    prepare_org_session(&env);
    let (service, _) = env.service();
    wait_for("the worker to grant edits", || service.policy().check().is_ok());
}

#[test]
fn no_org_denies_with_choose_org_reason() {
    let env = Env::new(org_scenario());
    let (service, account) = env.service();
    service.host().sign_in();
    wait_for("sign-in", || matches!(account.status(), Status::SignedIn(_)));
    assert_eq!(service.policy().check(), Err(CHOOSE_ORG.to_string()));
    let summary = service.host().summary();
    assert!(summary.signed_in);
    assert_eq!(summary.org, None);
    assert_eq!(summary.orgs.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), [ORG_A, ORG_B]);
}

#[test]
fn choose_org_then_entitled_user_can_edit() {
    let env = Env::new(org_scenario());
    let (service, account) = env.service();
    let host = service.host();
    host.sign_in();
    wait_for("sign-in", || matches!(account.status(), Status::SignedIn(_)));
    host.choose_org(ORG_A);
    wait_for("organisation A", || in_org(&account, ORG_A));
    wait_for("the worker to grant edits", || service.policy().check().is_ok());
    assert_eq!(host.summary().org.as_deref(), Some("Công ty Xây dựng Ví dụ"));
}

#[test]
fn revoked_device_denies_after_refresh() {
    let env = Env::new(org_scenario());
    prepare_org_session(&env);
    let (service, account) = env.service();
    account.refresh().unwrap();
    assert_eq!(service.policy().check(), Ok(()));
    let device = account.diagnostics().device_id.unwrap();
    assert!(env.server.revoke_device(&device));
    env.advance(13 * 3600);
    // In an organisation context the library records the revocation without returning an error.
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::DeviceRevoked));
    // A later refresh, as the worker runs one, makes online entitlements fresh again.
    env.advance(11 * 60);
    let _ = account.refresh();
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(REVOKED.to_string()));
}

#[test]
fn device_cap_denies_even_with_online_entitlements() {
    let env = Env::new(Scenario { max_devices: 0, ..org_scenario() });
    prepare_org_session(&env);
    let (service, account) = env.service();
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::DeviceCapReached));
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(DEVICE_CAP.to_string()));
}

#[test]
fn no_lease_denies_even_with_online_entitlements() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/v1/devices/activate", 422, "")], ..org_scenario() });
    prepare_org_session(&env);
    let (service, account) = env.service();
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::NotActivated));
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(NOT_ACTIVATED.to_string()));
}

#[test]
fn failed_sign_in_shows_a_notice() {
    let env = Env::new(org_scenario());
    let (service, _) = env.service_with(Arc::new(|_: &str| Err("no browser".to_owned())));
    let host = service.host();
    host.sign_in();
    wait_for("the browser notice", || host.summary().notice.as_deref() == Some(BROWSER_FAILED));

    env.server.add_fault(Fault::oauth("/id/token", 400, "invalid_grant"));
    let (service, _) = env.service();
    let host = service.host();
    host.sign_in();
    wait_for("the sign-in notice", || host.summary().notice.as_deref() == Some(SIGN_IN_FAILED));
}

#[test]
fn platform_down_keeps_cached_lease() {
    let env = Env::new(org_scenario());
    prepare_org_session(&env);
    for (method, path) in [
        ("GET", "/.well-known/openid-configuration"),
        ("POST", "/id/token"),
        ("GET", "/api/v1/id/me"),
        ("GET", "/v1/me/entitlements"),
        ("GET", "/v1/products/ubotbuild/feature-catalog"),
        ("POST", "/v1/devices/challenges"),
        ("POST", "/v1/devices/activate"),
        ("POST", "/v1/licenses/refresh"),
    ] {
        env.server.add_fault(Fault::delay(method, path, Duration::from_secs(5)));
    }
    let started = Instant::now();
    let (service, _) = env.service();
    assert!(started.elapsed() < Duration::from_secs(1), "start took {:?}", started.elapsed());
    assert_eq!(service.policy().check(), Ok(()));
}

#[test]
fn dev_lease_key_is_pinned_and_licensing_is_on() {
    let config = super::config::config_for("http://127.0.0.1:9", "0.9.0");
    assert!(config.offline_licensing);
    assert_eq!(config.product, Product::UbotBuild);
    assert_eq!(config.client_id, "ubotcad-desktop");
    let hex: String = pi_mock::dev_lease_public_key().iter().map(|b| format!("{b:02x}")).collect();
    let key = super::config::parse_dev_lease_key(&format!("{}:{hex}", pi_mock::DEV_LEASE_KID));
    assert_eq!(key, Some(PinnedKey { kid: pi_mock::DEV_LEASE_KID.into(), key: pi_mock::dev_lease_public_key() }));
    for bad in ["", "kid", ":00", "kid:00", &format!("kid:{hex}0"), &format!("kid:{}", "zz".repeat(32))] {
        assert_eq!(super::config::parse_dev_lease_key(bad), None, "{bad:?}");
    }
}

#[test]
fn reasons_have_vietnamese_translations() {
    use super::policy::RENEW_ONLINE;
    use cadcraft_ui_egui::i18n::tr;
    for (reason, vi) in [
        (SIGN_IN, "Đăng nhập PI ID để dùng bản Pro"),
        (CHOOSE_ORG, "Chọn tổ chức khi đăng nhập PI ID"),
        (RENEW_ONLINE, "Kết nối mạng để gia hạn"),
        (NO_PLAN, "Tài khoản chưa có gói dùng UbotCAD desktop"),
        (REVOKED, "Máy này đã bị gỡ khỏi tài khoản PI ID. Đăng nhập lại để dùng bản Pro"),
        (DEVICE_CAP, "Đã đạt số máy tối đa của gói PI ID. Gỡ một máy khác trong PI ID rồi thử lại"),
        (NOT_ACTIVATED, "Máy này chưa được kích hoạt bản Pro"),
        (super::policy::UNAVAILABLE, "Không dùng được giấy phép PI ID trên máy này; tạm khoá chỉnh sửa"),
        (SIGN_IN_FAILED, "Không hoàn tất được đăng nhập PI ID. Thử lại"),
        (BROWSER_FAILED, "Không mở được trình duyệt"),
    ] {
        assert_eq!(tr("vi", reason), vi);
    }
}

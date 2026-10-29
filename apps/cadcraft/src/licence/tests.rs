use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use erpcons_account::{Account, Clock, Config, LicenceState, MemorySecretStore, PinnedKey, Product, Status};
use pi_mock::{FREE_UBOTCAD, Fault, ORG_A, ORG_B, Scenario, Server, Tenant};

use super::LicenceService;
use super::host::{BROWSER_FAILED, CHECKING, SIGN_IN_FAILED};
use super::policy::{CHOOSE_ORG, DEVICE_CAP, Denial, NO_PLAN, NOT_ACTIVATED, REVOKED, SIGN_IN, denial_in};
use super::worker::Timing;

const T0: i64 = 1_792_828_800;
const USE: &str = "ubotbuild.cad-use";

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

    /// A service whose worker polls and resyncs within milliseconds; also counts its wake-ups,
    /// the first of which follows the start-up sync.
    fn fast_service(&self) -> (LicenceService, Arc<Account>, Arc<AtomicUsize>) {
        let account = self.account();
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = wakes.clone();
        let timing = Timing { poll: Duration::from_millis(20), resync_min: Duration::from_millis(20), resync_max: Duration::from_millis(50) };
        let wake = move || {
            counter.fetch_add(1, Ordering::SeqCst);
        };
        (LicenceService::start_timed(account.clone(), Arc::new(browser), wake, timing), account, wakes)
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

/// `ubotbuild.cad-use` granted to organisation A only.
fn org_scenario() -> Scenario {
    Scenario { tenants: vec![Tenant::new("ten_cad", ORG_A, "ubotbuild", &FREE_UBOTCAD)], ..Default::default() }
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
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::DeviceRevoked));
    // A later refresh, as the worker runs one, makes online entitlements fresh again.
    env.advance(11 * 60);
    let _ = account.refresh();
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(REVOKED.to_string()));
}

#[test]
fn not_entitled_denies_with_no_plan() {
    let env = Env::new(org_scenario());
    let account = env.account();
    account.sign_in(browser).unwrap();
    account.select_org(Some(ORG_B), browser).unwrap();
    let (service, account) = (LicenceService::start_with(account.clone(), Arc::new(browser), || {}), account);
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::NotEntitled));
    assert_eq!(service.policy().check(), Err(NO_PLAN.to_string()));
}

#[test]
fn device_cap_denies_even_with_online_entitlements() {
    let env = Env::new(Scenario { tenants: vec![Tenant::new("ten_cad", ORG_A, "ubotbuild", &FREE_UBOTCAD).device_cap(0)], ..Default::default() });
    prepare_org_session(&env);
    let (service, account) = env.service();
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::DeviceCapReached));
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(DEVICE_CAP.to_string()));
}

#[test]
fn no_lease_denies_even_with_online_entitlements() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/api/v1/id/devices/activate", 422, "")], ..org_scenario() });
    prepare_org_session(&env);
    let (service, account) = env.service();
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::NotActivated));
    assert!(account.allowed(USE), "online entitlements alone would still grant");
    assert_eq!(service.policy().check(), Err(NOT_ACTIVATED.to_string()));
}

#[test]
fn refused_edit_syncs_again_without_waiting_for_the_periodic_refresh() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/api/v1/id/devices/activate", 422, "")], ..org_scenario() });
    prepare_org_session(&env);
    let (service, account, wakes) = env.fast_service();
    wait_for("the start-up sync", || wakes.load(Ordering::SeqCst) > 0);
    assert_eq!(licence(&account), Some(LicenceState::NotActivated));
    assert!(service.host().summary().can_retry);
    // PI recovered; nothing asks for a refresh except the refused edit itself.
    env.server.clear_faults();
    assert_eq!(service.policy().check(), Err(NOT_ACTIVATED.to_string()));
    wait_for("the resync to activate this device", || service.policy().check().is_ok());
}

#[test]
fn retry_lifts_the_device_cap_once_a_device_is_removed() {
    let env = Env::new(Scenario { tenants: vec![Tenant::new("ten_cad", ORG_A, "ubotbuild", &FREE_UBOTCAD).device_cap(0)], ..Default::default() });
    prepare_org_session(&env);
    let (service, account, wakes) = env.fast_service();
    wait_for("the start-up sync", || wakes.load(Ordering::SeqCst) > 0);
    assert_eq!(service.policy().check(), Err(DEVICE_CAP.to_string()));
    let summary = service.host().summary();
    assert_eq!((summary.refused.as_deref(), summary.can_retry), (Some(DEVICE_CAP), true));
    // The user removed a device in PI ID: a plain sync keeps the block (lease.md §6), Retry lifts it.
    env.server.update_scenario(|s| s.tenants[0].device_cap = 2);
    let _ = account.refresh();
    assert_eq!(licence(&account), Some(LicenceState::DeviceCapReached));
    service.host().retry();
    wait_for("the retried activation", || service.policy().check().is_ok());
    wait_for("the checking notice to clear", || service.host().summary().notice.as_deref() != Some(CHECKING));
}

#[test]
fn expired_lease_reason_depends_on_being_online() {
    use erpcons_account::SignedIn;
    let status = |online: bool| {
        Status::SignedIn(Box::new(SignedIn {
            user: None,
            orgs: Vec::new(),
            active_org: Some(ORG_A.into()),
            licence: LicenceState::Inactive { lease_id: "l".into(), expires_at: T0 },
            licences: Default::default(),
            warning: None,
            online,
            session_only: false,
            reauth_required: false,
        }))
    };
    assert_eq!(denial_in(&status(false), |_| true), Some(Denial::RenewOffline));
    assert_eq!(denial_in(&status(true), |_| true), Some(Denial::RenewFailed));
    assert!(Denial::RenewFailed.sync_may_lift() && Denial::RenewOffline.sync_may_lift());
    assert!(!Denial::DeviceCap.sync_may_lift() && Denial::DeviceCap.retry_offered());
    assert!(!Denial::Revoked.retry_offered() && !Denial::SignIn.retry_offered() && !Denial::SessionOnly.retry_offered());
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
        ("GET", "/api/v1/id/me/entitlements"),
        ("GET", "/api/v1/id/devices/challenge"),
        ("POST", "/api/v1/id/devices/activate"),
        ("POST", "/api/v1/id/licenses/refresh"),
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
        (DEVICE_CAP, "Đã đạt số máy tối đa của gói PI ID. Gỡ một máy trong PI ID rồi bấm Thử lại ở Trợ giúp > Tài khoản"),
        (NOT_ACTIVATED, "Máy này chưa được kích hoạt bản Pro"),
        (super::policy::UNAVAILABLE, "Không dùng được giấy phép PI ID trên máy này; tạm khoá chỉnh sửa"),
        (SIGN_IN_FAILED, "Không hoàn tất được đăng nhập PI ID. Thử lại"),
        (BROWSER_FAILED, "Không mở được trình duyệt"),
        (super::policy::RENEW_FAILED, "Chưa gia hạn được giấy phép UbotCAD; UbotCAD sẽ tự thử lại"),
        (CHECKING, "Đang kiểm tra giấy phép UbotCAD với PI ID…"),
        ("Your UbotCAD licence expires in 1 day", "Giấy phép UbotCAD hết hạn sau 1 ngày"),
        ("Your UbotCAD licence expires in {days} days", "Giấy phép UbotCAD hết hạn sau {days} ngày"),
        ("Retry", "Thử lại"),
    ] {
        assert_eq!(tr("vi", reason), vi);
    }
}

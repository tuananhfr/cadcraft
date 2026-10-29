//! Where the desktop app finds PI and which keys verify its leases.

use erpcons_account::{Config, Product};

/// Production PI issuer; `None` until PI hosts the UbotCAD client.
pub const PI_ISSUER: Option<&str> = None;

// A release that cannot verify a lease would lock every user out of editing.
#[cfg(all(feature = "licensing", not(debug_assertions)))]
const _: () = assert!(!erpcons_account::PLATFORM_KEYS.is_empty() && PI_ISSUER.is_some());

/// The account configuration of this build, `None` when no PI issuer is known.
///
/// Debug builds read `UBOTCAD_PI_ISSUER` and `UBOTCAD_PI_DEV_LEASE_KEY` (`kid:hex32`) to run
/// against pi-mock; release builds compile neither in.
pub fn pi_config(client_version: &str) -> Option<Config> {
    #[cfg(debug_assertions)]
    {
        let issuer = env_var("UBOTCAD_PI_ISSUER").or_else(|| PI_ISSUER.map(str::to_owned))?;
        let mut config = config_for(&issuer, client_version);
        if let Some(value) = env_var("UBOTCAD_PI_DEV_LEASE_KEY") {
            match parse_dev_lease_key(&value) {
                Some(key) => config.platform_keys.push(key),
                None => log::warn!("UBOTCAD_PI_DEV_LEASE_KEY is not kid:<64 hex digits>; ignored"),
            }
        }
        Some(config)
    }
    #[cfg(not(debug_assertions))]
    {
        PI_ISSUER.map(|issuer| config_for(issuer, client_version))
    }
}

pub(super) fn config_for(issuer: &str, client_version: &str) -> Config {
    let mut config = Config::new(issuer, Product::UbotBuild, client_version);
    // Leases come from PI's device protocol (ADR 0012); the feature stays off in releases until
    // PI is deployed with it.
    config.offline_licensing = true;
    config
}

#[cfg(debug_assertions)]
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// `kid:hex32`: a lease-signing key id and its Ed25519 public key in 64 hex digits.
#[cfg(debug_assertions)]
pub(super) fn parse_dev_lease_key(value: &str) -> Option<erpcons_account::PinnedKey> {
    let (kid, hex) = value.split_once(':')?;
    // `from_str_radix` alone would accept a leading `+`.
    if kid.is_empty() || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(erpcons_account::PinnedKey { kid: kid.to_owned(), key })
}

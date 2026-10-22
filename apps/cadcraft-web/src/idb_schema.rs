pub const DB_NAME: &str = "cadcraft";
pub const DB_VERSION: u32 = 3;
pub const STORES: &[&str] =
    &["workspace", "snapshots", "damaged", "meta", "autosave", "autosave_index", "instances", "print-jobs", "print-profiles", "registry"];

pub fn missing_stores(mut contains: impl FnMut(&str) -> bool) -> Vec<&'static str> {
    STORES.iter().copied().filter(|name| !contains(name)).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn print_jobs_upgrade_keeps_profiles_and_autosave_intact() {
        let mut stores: BTreeMap<_, _> = STORES
            .iter()
            .copied()
            .filter(|name| !matches!(*name, "print-jobs" | "print-profiles" | "registry"))
            .map(|name| (name, format!("existing {name} data")))
            .collect();
        let previous = stores.clone();
        assert_eq!(DB_NAME, "cadcraft");
        assert_eq!(DB_VERSION, 3);
        assert_eq!(missing_stores(|name| stores.contains_key(name)), vec!["print-jobs", "print-profiles", "registry"]);
        for name in missing_stores(|name| stores.contains_key(name)) {
            stores.insert(name, String::new());
        }
        for (name, data) in previous {
            assert_eq!(stores.get(name), Some(&data));
        }
        assert!(missing_stores(|name| stores.contains_key(name)).is_empty());
    }

    #[test]
    fn idb_upgrade_keeps_v2_data_and_adds_profile_stores() {
        let old_stores = ["workspace", "snapshots", "damaged", "meta", "autosave", "autosave_index", "instances", "print-jobs"];
        let mut stores: BTreeMap<_, _> = old_stores.into_iter().map(|name| (name, format!("existing v2 {name} data"))).collect();
        let previous = stores.clone();
        assert_eq!(DB_NAME, "cadcraft");
        assert_eq!(DB_VERSION, 3);
        assert_eq!(missing_stores(|name| stores.contains_key(name)), vec!["print-profiles", "registry"]);
        for name in missing_stores(|name| stores.contains_key(name)) {
            stores.insert(name, String::new());
        }
        for (name, data) in previous {
            assert_eq!(stores.get(name), Some(&data));
        }
        assert!(stores.contains_key("print-profiles"));
        assert!(stores.contains_key("registry"));
        assert!(missing_stores(|name| stores.contains_key(name)).is_empty());
    }
}

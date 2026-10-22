pub const DB_NAME: &str = "cadcraft";
pub const DB_VERSION: u32 = 2;
pub const STORES: &[&str] = &["workspace", "snapshots", "damaged", "meta", "autosave", "autosave_index", "instances", "print-jobs"];

pub fn missing_stores(mut contains: impl FnMut(&str) -> bool) -> Vec<&'static str> {
    STORES.iter().copied().filter(|name| !contains(name)).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn print_jobs_upgrade_keeps_profiles_and_autosave_intact() {
        let mut stores: BTreeMap<_, _> =
            STORES.iter().copied().filter(|name| *name != "print-jobs").map(|name| (name, format!("existing {name} data"))).collect();
        let previous = stores.clone();
        assert_eq!(DB_NAME, "cadcraft");
        assert_eq!(DB_VERSION, 2);
        assert_eq!(missing_stores(|name| stores.contains_key(name)), vec!["print-jobs"]);
        for name in missing_stores(|name| stores.contains_key(name)) {
            stores.insert(name, String::new());
        }
        for (name, data) in previous {
            assert_eq!(stores.get(name), Some(&data));
        }
        assert!(missing_stores(|name| stores.contains_key(name)).is_empty());
    }
}

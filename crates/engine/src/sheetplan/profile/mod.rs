pub mod merge;
pub mod migrate;
pub mod schema;
pub mod store;
#[cfg(not(target_arch = "wasm32"))]
pub mod store_fs;

pub use merge::{check_change, effective, effective_with_issues, naming_tokens};
pub use migrate::{MigrateError, migrate};
pub use schema::*;
pub use store::{MemoryPrintProfileStore, PrintProfileStore, drawing_key, load_doc};
#[cfg(not(target_arch = "wasm32"))]
pub use store_fs::DirPrintProfileStore;

impl crate::Session {
    pub fn set_print_profile_store(&mut self, store: Box<dyn PrintProfileStore>) {
        self.print_profile_store = store;
    }

    pub fn write_print_profile(&mut self, doc: &ProfileDoc) -> Result<(), String> {
        merge::validate_doc(doc)?;
        let json = serde_json::to_string(doc).map_err(|error| error.to_string())?;
        self.print_profile_store.write(doc.level, &doc.id, &json)
    }

    pub fn effective_print_profile(&mut self) -> EffectiveProfile {
        let mut keys = vec![(ProfileLevel::Company, "default".to_string())];
        if let Some(state) = self.docs.get(self.active) {
            keys.push((ProfileLevel::Drawing, drawing_key(state.path.as_deref(), &state.title)));
        }
        keys.push((ProfileLevel::User, "default".to_string()));
        let mut docs = Vec::new();
        for (level, id) in keys {
            let (doc, notice) = load_doc(self.print_profile_store.as_mut(), level, &id);
            if let Some(notice) = notice
                && !self.log.contains(&notice)
            {
                self.echo(notice);
            }
            if let Some(doc) = doc {
                docs.push(doc);
            }
        }
        let (profile, issues) = effective_with_issues(&docs);
        for issue in issues {
            if !self.log.contains(&issue.message) {
                self.echo(issue.message);
            }
        }
        profile
    }
}

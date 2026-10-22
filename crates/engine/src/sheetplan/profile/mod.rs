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

    pub fn set_drawing_project(&mut self, project_id: Option<&str>) -> Result<(), String> {
        let state = self.docs.get(self.active).ok_or("there is no drawing open")?;
        let id = drawing_key(state.path.as_deref(), &state.title);
        if let Some(project_id) = project_id {
            store::validate_id(ProfileLevel::Project, project_id)?;
        }
        let mut doc = self.load_print_profile_doc(ProfileLevel::Drawing, &id).unwrap_or_else(|| ProfileDoc {
            schema_version: PRINT_PROFILE_SCHEMA,
            level: ProfileLevel::Drawing,
            id,
            project_id: None,
            revision: 0,
            settings: serde_json::json!({}),
            locks: Vec::new(),
            updated_ms: 0,
        });
        doc.project_id = project_id.map(str::to_owned);
        self.write_print_profile(&doc)
    }

    fn load_print_profile_doc(&mut self, level: ProfileLevel, id: &str) -> Option<ProfileDoc> {
        let (doc, notice) = load_doc(self.print_profile_store.as_mut(), level, id);
        if let Some(notice) = notice
            && !self.log.contains(&notice)
        {
            self.echo(notice);
        }
        doc
    }

    pub fn effective_print_profile(&mut self) -> EffectiveProfile {
        let mut docs = Vec::new();
        if let Some(company) = self.load_print_profile_doc(ProfileLevel::Company, "default") {
            docs.push(company);
        }
        let drawing_id = self.docs.get(self.active).map(|state| drawing_key(state.path.as_deref(), &state.title));
        if let Some(drawing) = drawing_id.and_then(|id| self.load_print_profile_doc(ProfileLevel::Drawing, &id)) {
            if let Some(project_id) = drawing.project_id.as_deref()
                && let Some(project) = self.load_print_profile_doc(ProfileLevel::Project, project_id)
            {
                docs.push(project);
            }
            docs.push(drawing);
        }
        if let Some(user) = self.load_print_profile_doc(ProfileLevel::User, "default") {
            docs.push(user);
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

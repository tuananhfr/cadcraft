use cadcraft_ubot::Catalog;

/// The session's U-BOT catalog: the built-in draft catalog with the last loaded overlay on top.
#[derive(Debug, Clone)]
pub struct UbotState {
    pub catalog: Catalog,
    /// Where the overlay came from (`path` or `"text"`); `None` while only the built-in catalog is in use.
    pub overlay_source: Option<String>,
}

impl Default for UbotState {
    fn default() -> Self {
        Self { catalog: Catalog::builtin(), overlay_source: None }
    }
}

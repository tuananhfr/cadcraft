use cadcraft_ubot::Catalog;

/// The session's U-BOT catalog: the built-in draft catalog with the last loaded overlay on top.
#[derive(Debug, Clone)]
pub struct UbotState {
    pub catalog: Catalog,
    /// Where the overlay came from (`path` or `"text"`); `None` while only the built-in catalog is in use.
    pub overlay_source: Option<String>,
    /// The layout id UBFILL made up for an untagged (or copied) boundary, so a commit matches its preview.
    pub(crate) fresh_layout: Option<FreshLayout>,
}

/// A generated layout id and what it was generated for.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FreshLayout {
    pub doc_uid: u64,
    pub boundary: String,
    pub geometry_hash: String,
    pub previous: Option<String>,
    pub layout_id: String,
}

impl Default for UbotState {
    fn default() -> Self {
        Self { catalog: Catalog::builtin(), overlay_source: None, fresh_layout: None }
    }
}

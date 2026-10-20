//! Workspace profiles: the serialisable description of a user's layout (ribbon, tool bars, panels,
//! shortcuts), its validation and the built-in presets. Pure data; applying a profile to the app
//! lives elsewhere.

pub mod presets;
pub mod schema;
pub mod validate;

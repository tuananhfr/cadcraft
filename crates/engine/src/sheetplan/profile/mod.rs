pub mod merge;
pub mod schema;

pub use merge::{check_change, effective, effective_with_issues, naming_tokens};
pub use schema::*;

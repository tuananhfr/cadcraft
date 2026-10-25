//! U-BOT toolkit core: catalog model and deterministic hashing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod catalog;
mod error;
mod hash;
mod model;

pub use catalog::Catalog;
pub use error::UbotError;
pub use hash::input_hash;
pub use model::{Anchor, ApprovalStatus, BlockMapping, Product, RulePack};

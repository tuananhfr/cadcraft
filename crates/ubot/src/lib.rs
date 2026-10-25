//! U-BOT toolkit core: catalog model and deterministic hashing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod catalog;
mod error;
mod geometry;
mod hash;
mod model;

pub use catalog::Catalog;
pub use error::UbotError;
pub use geometry::{
    EdgeIndex, Footprint, Ring, RingInput, distance_footprint_ring_edges, distance_footprint_ring_edges_with, footprint, footprint_intersects_ring,
    footprint_intersects_ring_with, point_in_ring, ring_contains_footprint, ring_contains_footprint_with, ring_from_input,
};
pub use hash::input_hash;
pub use model::{Anchor, ApprovalStatus, BlockMapping, Product, RulePack};

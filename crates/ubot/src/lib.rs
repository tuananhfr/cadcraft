//! U-BOT toolkit core: catalog model, deterministic hashing, layout geometry and grid fill.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod catalog;
mod check;
mod error;
mod fill;
mod geometry;
mod hash;
mod model;
mod placement;
mod protocol;
mod quantity;
mod replace;

pub use catalog::Catalog;
pub use check::{CheckInstance, CheckRequest, Issue, IssueCode, LayoutInput, Severity, check};
pub use error::UbotError;
pub use fill::{
    Candidate, ExistingInstance, FillRequest, FillResult, FillSummary, LayoutRecord, MAX_CLEARANCE_MM, Placement, Reason, Status, UpdatePlacement,
    ZoneInput, ZoneReach, ZoneType, fill, geometry_hash,
};
pub use geometry::{
    EdgeIndex, Footprint, Ring, RingInput, distance_footprint_ring_edges, distance_footprint_ring_edges_with, footprint, footprint_intersects_ring,
    footprint_intersects_ring_with, point_in_ring, ring_contains_footprint, ring_contains_footprint_with, ring_from_input,
};
pub use hash::input_hash;
pub use model::{Anchor, ApprovalStatus, BlockMapping, Product, RulePack};
pub use placement::{center_from_insert, center_from_insert_xy, has_anchor_offset, insert_from_center, insert_from_center_xy, relative_scale};
pub use protocol::{MAX_INPUT_BYTES, handle_json, handle_json_status};
pub use quantity::{CountItem, CountRow, CountSource, Counts, MAX_COUNT_ITEMS, count, count_in};
pub use replace::{ReplaceChange, ReplaceRequest, ReplaceResult, replace};

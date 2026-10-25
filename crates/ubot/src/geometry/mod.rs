//! Planar geometry of the U-BOT layout: validated rings, box footprints and a bucket index over edges.

mod footprint;
mod index;
mod ring;
mod segment;
mod tessellate;

pub use footprint::{
    Footprint, distance_footprint_ring_edges, distance_footprint_ring_edges_with, footprint, footprint_intersects_ring,
    footprint_intersects_ring_with, ring_contains_footprint, ring_contains_footprint_with,
};
pub use index::EdgeIndex;
pub use ring::{Ring, RingInput, point_in_ring, ring_from_input};

/// Hard cap on vertices of one ring after tessellation.
pub(crate) const MAX_RING_VERTICES: usize = 20_000;
/// Points closer than this (mm) are the same point; contact tests use it as their tolerance.
pub(crate) const POINT_TOL_MM: f64 = 1e-6;

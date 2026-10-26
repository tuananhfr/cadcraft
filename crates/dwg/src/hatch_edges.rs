//! HATCH ellipse edge parameters between acadrust's units and DXF's.
//!
//! acadrust 0.6.4 keeps a hatch ellipse edge's start and end parameters in radians, as DWG stores
//! them, but its DXF reader and writer pass groups 50/51 through unconverted, while DXF holds
//! them in degrees (as for arc edges, which acadrust does convert). Read raw, an AutoCAD DWG's
//! edge spans a fraction of a degree and its points collapse onto the major axis ends.

use acadrust::entities::{BoundaryEdge, EntityType};

/// Which way to convert the ellipse edges' parameters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Units {
    /// Radians to degrees, before acadrust's DXF writer runs.
    ToDegrees,
    /// Degrees to radians, after acadrust's DXF reader ran.
    ToRadians,
}

/// Convert every hatch ellipse edge's parameters in `doc`; other entities are left untouched.
pub(crate) fn convert_ellipse_edges(doc: &mut acadrust::CadDocument, units: Units) {
    let hatches: Vec<acadrust::Handle> = doc
        .entities()
        .filter_map(|e| match e {
            EntityType::Hatch(h) if h.paths.iter().flat_map(|p| &p.edges).any(|e| matches!(e, BoundaryEdge::EllipticArc(_))) => Some(h.common.handle),
            _ => None,
        })
        .collect();
    let convert = |a: f64| match units {
        Units::ToDegrees => a.to_degrees(),
        Units::ToRadians => a.to_radians(),
    };
    for handle in hatches {
        let Some(EntityType::Hatch(h)) = doc.get_entity_mut(handle) else { continue };
        for edge in h.paths.iter_mut().flat_map(|p| &mut p.edges) {
            if let BoundaryEdge::EllipticArc(a) = edge {
                a.start_angle = convert(a.start_angle);
                a.end_angle = convert(a.end_angle);
            }
        }
    }
}

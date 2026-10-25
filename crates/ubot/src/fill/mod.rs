//! Grid fill (GEOM-03/04): lays boxes on a grid inside a slab boundary, classifies every cell and diffs the result
//! against the boxes already in the drawing so re-runs are idempotent.

mod classify;
mod diff;
mod grid;

use cadcraft_geom::Vec2;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::UbotError;
use crate::geometry::{RingInput, ring_from_input};
use crate::hash::input_hash;
use crate::model::{ApprovalStatus, Product, RulePack};
use classify::{Domain, NEAR_BAND_MM, Obstacles, Role};
use diff::Placer;
use grid::{Grid, GridSpec};

/// Boundary, hole and zone vertices together, after arc subdivision (spec §6).
const MAX_VERTICES: usize = 20_000;
const MAX_LAYOUT_ID_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneType {
    Exclusion,
    Solid,
    Column,
    Wall,
    Opening,
    AltProduct,
}

/// Zone ring in drawing units; a CIRCLE arrives as a two-vertex ring with bulge 1 on both halves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoneInput {
    pub id: String,
    pub ring: RingInput,
    pub zone_type: ZoneType,
    /// `None` falls back to the request's, then the rulepack's exclusion clearance.
    #[serde(default)]
    pub clearance_mm: Option<f64>,
    #[serde(default)]
    pub alt_product_id: Option<String>,
}

/// A box INSERT already in the drawing, in drawing units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingInstance {
    pub handle: String,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub layout_id: Option<String>,
    #[serde(default)]
    pub product_id: Option<String>,
    pub block_name: String,
    pub insert: [f64; 2],
    pub rotation_deg: f64,
    /// Uniform INSERT scale.
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillRequest {
    /// Millimetres per drawing unit.
    pub unit_scale: f64,
    pub layout_id: String,
    #[serde(default)]
    pub previous_layout_id: Option<String>,
    /// Pull manually moved boxes back to their computed position.
    #[serde(default)]
    pub reset: bool,
    pub boundary: RingInput,
    #[serde(default)]
    pub holes: Vec<RingInput>,
    #[serde(default)]
    pub zones: Vec<ZoneInput>,
    pub product_id: String,
    #[serde(default)]
    pub rulepack_id: Option<String>,
    #[serde(default)]
    pub rib_mm: Option<f64>,
    #[serde(default)]
    pub pitch_x_mm: Option<f64>,
    #[serde(default)]
    pub pitch_y_mm: Option<f64>,
    pub angle_deg: f64,
    /// Centre of cell (0, 0) in drawing units.
    #[serde(default)]
    pub origin: Option<[f64; 2]>,
    #[serde(default)]
    pub edge_clearance_mm: Option<f64>,
    #[serde(default)]
    pub exclusion_clearance_mm: Option<f64>,
    #[serde(default)]
    pub existing: Vec<ExistingInstance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Valid,
    Invalid,
    Uncertain,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
    OutsideBoundary,
    InHole,
    EdgeClearance,
    InZone,
    ZoneClearance,
    OverlapsExisting,
    CurvedEdgeTolerance,
    NearLimit,
    ManualMoved,
}

/// A box to write; `insert` and `center` are drawing units, `insert` already has the block anchor applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub instance_id: String,
    pub cell: [i64; 2],
    pub product_id: String,
    pub block_name: String,
    pub insert: [f64; 2],
    pub center: [f64; 2],
    pub rotation_deg: f64,
}

/// A cell worth reporting; `center` is the cell centre in drawing units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub cell: [i64; 2],
    pub center: [f64; 2],
    pub status: Status,
    pub reasons: Vec<Reason>,
    pub product_id: String,
}

/// What the adapter stores on the boundary; `origin` is drawing units, `revision` is bumped by the adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutRecord {
    pub layout_id: String,
    pub revision: u32,
    pub product_id: String,
    /// `"<id>@<version>"`.
    pub rulepack: String,
    pub origin: [f64; 2],
    pub angle_deg: f64,
    pub pitch_x_mm: f64,
    pub pitch_y_mm: f64,
    pub unit_scale: f64,
    pub geometry_hash: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillSummary {
    pub valid: usize,
    pub invalid: usize,
    pub uncertain: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub kept: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlacement {
    pub handle: String,
    pub placement: Placement,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FillResult {
    pub input_hash: String,
    pub layout: LayoutRecord,
    pub add: Vec<Placement>,
    pub update: Vec<UpdatePlacement>,
    /// Handles to erase.
    pub remove: Vec<String>,
    /// Handles left untouched.
    pub keep: Vec<String>,
    /// Invalid and uncertain cells plus `Info(ManualMoved)`, top-down then left to right.
    pub problems: Vec<Candidate>,
    pub summary: FillSummary,
    pub warnings: Vec<String>,
}

/// `input_hash` of the boundary, holes and zones with coordinates converted to mm.
pub fn geometry_hash(unit_scale: f64, boundary: &RingInput, holes: &[RingInput], zones: &[ZoneInput]) -> Result<String, UbotError> {
    positive(unit_scale, "unitScale")?;
    let mm = |r: &RingInput| -> Result<RingInput, UbotError> {
        let vertices: Vec<[f64; 3]> = r.vertices.iter().map(|v| [v[0] * unit_scale, v[1] * unit_scale, v[2]]).collect();
        if vertices.iter().flatten().all(|c| c.is_finite()) {
            Ok(RingInput { vertices })
        } else {
            Err(UbotError::BadInput("ring vertex is not a finite number in mm".into()))
        }
    };
    let zones = zones.iter().map(|z| Ok(ZoneInput { ring: mm(&z.ring)?, ..z.clone() })).collect::<Result<Vec<ZoneInput>, UbotError>>()?;
    let view = GeometryView { boundary: mm(boundary)?, holes: holes.iter().map(mm).collect::<Result<_, _>>()?, zones };
    Ok(input_hash(&view))
}

#[derive(Serialize)]
struct GeometryView {
    boundary: RingInput,
    holes: Vec<RingInput>,
    zones: Vec<ZoneInput>,
}

/// The request minus `existing`: what the drawing holds today does not change the layout being asked for.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestView<'a> {
    unit_scale: f64,
    layout_id: &'a str,
    previous_layout_id: Option<&'a str>,
    reset: bool,
    boundary: &'a RingInput,
    holes: &'a [RingInput],
    zones: &'a [ZoneInput],
    product_id: &'a str,
    rulepack_id: Option<&'a str>,
    rib_mm: Option<f64>,
    pitch_x_mm: Option<f64>,
    pitch_y_mm: Option<f64>,
    angle_deg: f64,
    origin: Option<[f64; 2]>,
    edge_clearance_mm: Option<f64>,
    exclusion_clearance_mm: Option<f64>,
}

impl<'a> RequestView<'a> {
    fn of(r: &'a FillRequest) -> Self {
        RequestView {
            unit_scale: r.unit_scale,
            layout_id: &r.layout_id,
            previous_layout_id: r.previous_layout_id.as_deref(),
            reset: r.reset,
            boundary: &r.boundary,
            holes: &r.holes,
            zones: &r.zones,
            product_id: &r.product_id,
            rulepack_id: r.rulepack_id.as_deref(),
            rib_mm: r.rib_mm,
            pitch_x_mm: r.pitch_x_mm,
            pitch_y_mm: r.pitch_y_mm,
            angle_deg: r.angle_deg,
            origin: r.origin,
            edge_clearance_mm: r.edge_clearance_mm,
            exclusion_clearance_mm: r.exclusion_clearance_mm,
        }
    }
}

fn finite(v: f64, name: &str) -> Result<f64, UbotError> {
    if v.is_finite() { Ok(v) } else { Err(UbotError::BadInput(format!("{name} must be a finite number"))) }
}

fn positive(v: f64, name: &str) -> Result<f64, UbotError> {
    if finite(v, name)? > 0.0 { Ok(v) } else { Err(UbotError::BadInput(format!("{name} must be > 0"))) }
}

fn non_negative(v: f64, name: &str) -> Result<f64, UbotError> {
    if finite(v, name)? >= 0.0 { Ok(v) } else { Err(UbotError::BadInput(format!("{name} must be >= 0"))) }
}

/// R3.6: 1..=64 characters of `[0-9A-Za-z_-]`.
fn check_layout_id(id: &str, name: &str) -> Result<(), UbotError> {
    let ok = !id.is_empty() && id.len() <= MAX_LAYOUT_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok { Ok(()) } else { Err(UbotError::BadInput(format!("{name} must be 1-{MAX_LAYOUT_ID_LEN} characters of [0-9A-Za-z_-]"))) }
}

/// Rib, pitch and clearances after falling back to the rulepack.
struct Settings {
    rib_mm: f64,
    pitch_mm: Vec2,
    edge_clearance_mm: f64,
    exclusion_clearance_mm: f64,
}

fn settings(req: &FillRequest, product: &Product, rulepack: &RulePack) -> Result<Settings, UbotError> {
    let rib_mm = req.rib_mm.map(|v| non_negative(v, "ribMm")).transpose()?.unwrap_or(rulepack.rib_mm_default);
    let pitch = |given: Option<f64>, name: &str, size: f64| -> Result<f64, UbotError> {
        let p = given.map(|v| positive(v, name)).transpose()?.unwrap_or(size + rib_mm);
        // A pitch below the box size would lay boxes over each other.
        if p < size { Err(UbotError::BadInput(format!("{name} {p} is smaller than the box ({size} mm)"))) } else { Ok(p) }
    };
    let clearance = |given: Option<f64>, name: &str, default: f64| given.map(|v| non_negative(v, name)).transpose().map(|v| v.unwrap_or(default));
    Ok(Settings {
        rib_mm,
        pitch_mm: Vec2::new(pitch(req.pitch_x_mm, "pitchXMm", product.width_mm)?, pitch(req.pitch_y_mm, "pitchYMm", product.length_mm)?),
        edge_clearance_mm: clearance(req.edge_clearance_mm, "edgeClearanceMm", rulepack.edge_clearance_mm)?,
        exclusion_clearance_mm: clearance(req.exclusion_clearance_mm, "exclusionClearanceMm", rulepack.exclusion_clearance_mm)?,
    })
}

fn build_domain<'c>(req: &FillRequest, catalog: &'c Catalog, main: &Product, s: &Settings) -> Result<Domain<'c>, UbotError> {
    let raw = req.holes.iter().chain(req.zones.iter().map(|z| &z.ring)).fold(req.boundary.vertices.len(), |n, r| n.saturating_add(r.vertices.len()));
    if raw > MAX_VERTICES {
        return Err(UbotError::Limit(format!("{raw} boundary, hole and zone vertices, the limit is {MAX_VERTICES}")));
    }
    let ring = |r: &RingInput| ring_from_input(r, req.unit_scale, NEAR_BAND_MM);
    let mut rings = vec![ring(&req.boundary)?];
    let mut roles = vec![Role::Boundary];
    for h in &req.holes {
        rings.push(ring(h)?);
        roles.push(Role::Hole);
    }
    for z in &req.zones {
        let clearance_mm = match z.clearance_mm {
            Some(c) => non_negative(c, &format!("zone {} clearanceMm", z.id))?,
            None => s.exclusion_clearance_mm,
        };
        roles.push(match z.zone_type {
            ZoneType::Exclusion | ZoneType::Solid | ZoneType::Column | ZoneType::Wall => Role::Blocker { clearance_mm },
            ZoneType::Opening => Role::Opening { clearance_mm },
            ZoneType::AltProduct => Role::Alt { product: alt_product(z, catalog, main)? },
        });
        rings.push(ring(&z.ring)?);
    }
    let total = rings.iter().fold(0usize, |n, r| n.saturating_add(r.points.len()));
    if total > MAX_VERTICES {
        return Err(UbotError::Limit(format!("{total} vertices after arc subdivision, the limit is {MAX_VERTICES}")));
    }
    Ok(Domain { rings, roles, edge_clearance_mm: s.edge_clearance_mm })
}

fn alt_product<'c>(z: &ZoneInput, catalog: &'c Catalog, main: &Product) -> Result<&'c Product, UbotError> {
    let id = z.alt_product_id.as_deref().ok_or_else(|| UbotError::BadInput(format!("alt_product zone {} has no altProductId", z.id)))?;
    let p = catalog.product(id).ok_or_else(|| UbotError::UnknownProduct(id.to_string()))?;
    if p.width_mm != main.width_mm || p.length_mm != main.length_mm {
        return Err(UbotError::BadInput(format!(
            "alt product {id} is {}x{} mm, the layout product {} is {}x{} mm",
            p.width_mm, p.length_mm, main.id, main.width_mm, main.length_mm
        )));
    }
    Ok(p)
}

fn draft_warnings(rulepack: &RulePack, products: &[&Product]) -> Vec<String> {
    let mut out = Vec::new();
    if rulepack.approval_status == ApprovalStatus::Draft {
        out.push(format!("RULEPACK_DRAFT {}@{}", rulepack.id, rulepack.version));
    }
    for p in products.iter().filter(|p| p.approval_status == ApprovalStatus::Draft) {
        out.push(format!("PRODUCT_DRAFT {}", p.id));
    }
    out
}

/// Lays the grid, classifies each cell and diffs against `req.existing`; nothing is written, the adapter applies the result.
pub fn fill(req: &FillRequest, catalog: &Catalog) -> Result<FillResult, UbotError> {
    check_layout_id(&req.layout_id, "layoutId")?;
    if let Some(prev) = &req.previous_layout_id {
        check_layout_id(prev, "previousLayoutId")?;
    }
    positive(req.unit_scale, "unitScale")?;
    finite(req.angle_deg, "angleDeg")?;
    let origin_mm = match req.origin {
        Some([x, y]) => Some(Vec2::new(finite(x, "origin")?, finite(y, "origin")?) * req.unit_scale),
        None => None,
    };
    if origin_mm.is_some_and(|o| !o.is_finite()) {
        return Err(UbotError::BadInput("origin overflows in mm".into()));
    }
    let main = catalog.product(&req.product_id).ok_or_else(|| UbotError::UnknownProduct(req.product_id.clone()))?;
    let rulepack = catalog.rulepack(req.rulepack_id.as_deref()).ok_or_else(|| {
        UbotError::BadInput(match &req.rulepack_id {
            Some(id) => format!("unknown rulepack '{id}'"),
            None => "the catalog has no rulepack".into(),
        })
    })?;
    let s = settings(req, main, rulepack)?;
    let domain = build_domain(req, catalog, main, &s)?;
    let boundary = domain.rings.first().ok_or_else(|| UbotError::Geometry("missing boundary".into()))?;
    let spec =
        GridSpec { angle_deg: req.angle_deg, origin_mm, size_mm: Vec2::new(main.width_mm, main.length_mm), rib_mm: s.rib_mm, pitch_mm: s.pitch_mm };
    let grid = Grid::new(&boundary.points, &spec)?;
    let ids = [Some(req.layout_id.as_str()), req.previous_layout_id.as_deref()];
    let (owned, others) = diff::split_existing(&req.existing, ids, catalog, main, req.unit_scale, boundary)?;
    let obstacles = Obstacles::new(&grid, others)?;
    let cells = classify::classify(&grid, &domain, &obstacles, main)?;

    let mut products = vec![main];
    for role in &domain.roles {
        if let Role::Alt { product } = role
            && !products.iter().any(|p| p.id == product.id)
        {
            products.push(product);
        }
    }
    let placer = Placer::new(catalog, &req.layout_id, req.unit_scale, &products);
    let d = diff::diff(&cells, owned, req.reset, &placer, &grid)?;

    let mut problems: Vec<Candidate> = cells
        .iter()
        .filter(|c| matches!(c.status, Status::Invalid | Status::Uncertain))
        .map(|c| Candidate {
            cell: c.cell,
            center: placer.drawing(c.center_mm),
            status: c.status,
            reasons: c.reasons.clone(),
            product_id: c.product.id.clone(),
        })
        .chain(d.blocked.iter().cloned())
        .chain(d.moved)
        .collect();
    // Stable: a cell's own problem stays ahead of its ManualMoved note.
    problems.sort_by_key(|p| (p.cell[1], p.cell[0]));
    let count = |s: Status| problems.iter().filter(|p| p.status == s).count();
    let valid = cells.iter().filter(|c| c.status == Status::Valid).count().saturating_sub(d.blocked.len());
    let summary = FillSummary {
        valid,
        invalid: count(Status::Invalid),
        uncertain: count(Status::Uncertain),
        added: d.add.len(),
        updated: d.update.len(),
        removed: d.remove.len(),
        kept: d.keep.len(),
    };
    let mut warnings = draft_warnings(rulepack, &products);
    warnings.extend(d.warnings);
    Ok(FillResult {
        input_hash: input_hash(&RequestView::of(req)),
        layout: LayoutRecord {
            layout_id: req.layout_id.clone(),
            revision: 0,
            product_id: main.id.clone(),
            rulepack: format!("{}@{}", rulepack.id, rulepack.version),
            origin: placer.drawing(grid.origin_world()),
            angle_deg: req.angle_deg,
            pitch_x_mm: s.pitch_mm.x,
            pitch_y_mm: s.pitch_mm.y,
            unit_scale: req.unit_scale,
            geometry_hash: geometry_hash(req.unit_scale, &req.boundary, &req.holes, &req.zones)?,
        },
        add: d.add,
        update: d.update,
        remove: d.remove,
        keep: d.keep,
        problems,
        summary,
        warnings,
    })
}

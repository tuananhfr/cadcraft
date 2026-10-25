//! Grid frame and cell ranges, plus the scanline that answers "is this point inside ring r" for a whole row at once.

use cadcraft_geom::{Bounds2, Vec2};

use crate::error::UbotError;

/// Cells tested per run (spec §6).
pub(super) const MAX_CELLS: usize = 100_000;
/// Cell indices beyond this lose f64 precision in `origin + i·pitch`; such an origin is far from the slab anyway.
const MAX_CELL_INDEX: f64 = 1e12;
/// Total scanline and classification steps (bucket visits, edge tests); only adversarial geometry or clearances
/// reaching thousands of edges per cell get near it, and it is spent in well under a second.
const MAX_WORK: u64 = 50_000_000;

pub(super) struct GridSpec {
    pub angle_deg: f64,
    /// Centre of cell (0, 0) in mm; `None` derives it from the boundary bounds.
    pub origin_mm: Option<Vec2>,
    pub size_mm: Vec2,
    pub rib_mm: f64,
    pub pitch_mm: Vec2,
}

/// Grid frame: `u` runs along the box width, `v` along its length. Row j sits `j` pitches below the origin, so j grows
/// downwards and i to the right.
pub(super) struct Grid {
    u: Vec2,
    v: Vec2,
    /// Centre of cell (0, 0) in grid coordinates.
    origin: Vec2,
    pub angle_deg: f64,
    pub size_mm: Vec2,
    pub pitch_mm: Vec2,
    pub i_range: (i64, i64),
    pub j_range: (i64, i64),
    cols: usize,
    rows: usize,
}

impl Grid {
    /// Frame and pitch only: no cell range, origin at the frame origin.
    fn bare(angle_deg: f64, size_mm: Vec2, pitch_mm: Vec2) -> Grid {
        let a = angle_deg.to_radians();
        Grid {
            u: Vec2::new(a.cos(), a.sin()),
            v: Vec2::new(-a.sin(), a.cos()),
            origin: Vec2::ZERO,
            angle_deg,
            size_mm,
            pitch_mm,
            i_range: (0, -1),
            j_range: (0, -1),
            cols: 0,
            rows: 0,
        }
    }

    /// Covers every cell whose footprint overlaps the bounds of `boundary` (mm); errors past [`MAX_CELLS`].
    pub fn new(boundary: &[Vec2], spec: &GridSpec) -> Result<Grid, UbotError> {
        let half = spec.size_mm * 0.5;
        let mut grid = Grid::bare(spec.angle_deg, spec.size_mm, spec.pitch_mm);
        let gb = Bounds2::from_points(boundary.iter().map(|p| grid.to_grid(*p)));
        if gb.is_empty() || !(gb.min.is_finite() && gb.max.is_finite()) {
            return Err(UbotError::Geometry("boundary has no finite extent".into()));
        }
        grid.origin = match spec.origin_mm {
            Some(o) => grid.to_grid(o),
            None => Vec2::new(gb.min.x + spec.rib_mm / 2.0 + half.x, gb.max.y - (spec.rib_mm / 2.0 + half.y)),
        };
        let (o, p) = (grid.origin, spec.pitch_mm);
        // No clamp at 0 for the default origin: with a pitch below width + rib/2, cell −1 still straddles the bounds and
        // must be reported (R3.9).
        let i_range = open_span((gb.min.x - half.x - o.x) / p.x, (gb.max.x + half.x - o.x) / p.x)?;
        let j_range = open_span((o.y - half.y - gb.max.y) / p.y, (o.y + half.y - gb.min.y) / p.y)?;
        let (cols, rows) = (span_len(i_range)?, span_len(j_range)?);
        let cells = cols.checked_mul(rows).ok_or_else(too_many_cells)?;
        if cells > MAX_CELLS {
            return Err(too_many_cells());
        }
        grid.i_range = i_range;
        grid.j_range = j_range;
        grid.cols = cols;
        grid.rows = rows;
        Ok(grid)
    }

    pub fn to_grid(&self, p: Vec2) -> Vec2 {
        Vec2::new(p.dot(self.u), p.dot(self.v))
    }

    pub fn to_world(&self, g: Vec2) -> Vec2 {
        self.u * g.x + self.v * g.y
    }

    pub fn origin_world(&self) -> Vec2 {
        self.to_world(self.origin)
    }

    /// Cell centre in grid coordinates.
    pub fn center(&self, i: i64, j: i64) -> Vec2 {
        Vec2::new(self.origin.x + i as f64 * self.pitch_mm.x, self.origin.y - j as f64 * self.pitch_mm.y)
    }

    pub fn row_y(&self, j: i64) -> f64 {
        self.origin.y - j as f64 * self.pitch_mm.y
    }

    pub fn cell_count(&self) -> usize {
        self.cols * self.rows
    }

    /// Dense row-major index of an in-range cell.
    pub fn index(&self, i: i64, j: i64) -> Option<usize> {
        let (di, dj) = (i.checked_sub(self.i_range.0)?, j.checked_sub(self.j_range.0)?);
        let (di, dj) = (usize::try_from(di).ok()?, usize::try_from(dj).ok()?);
        (di < self.cols && dj < self.rows).then(|| dj * self.cols + di)
    }

    /// In-range cells whose footprint bounds overlap `bounds` (grid coordinates) with positive area: `(i_lo, i_hi, j_lo, j_hi)`.
    pub fn cells_overlapping(&self, bounds: &Bounds2) -> Option<(i64, i64, i64, i64)> {
        if bounds.is_empty() || !(bounds.min.is_finite() && bounds.max.is_finite()) {
            return None;
        }
        let (o, p, half) = (self.origin, self.pitch_mm, self.size_mm * 0.5);
        let i = clip_span((bounds.min.x - half.x - o.x) / p.x, (bounds.max.x + half.x - o.x) / p.x, self.i_range)?;
        let j = clip_span((o.y - half.y - bounds.max.y) / p.y, (o.y + half.y - bounds.min.y) / p.y, self.j_range)?;
        Some((i.0, i.1, j.0, j.1))
    }
}

/// World position (mm) of the centre of cell `(i, j)` of a stored layout whose cell (0, 0) is centred at `origin_world`.
pub(crate) fn cell_center_world(angle_deg: f64, origin_world: Vec2, pitch_mm: Vec2, i: i64, j: i64) -> Vec2 {
    let mut grid = Grid::bare(angle_deg, Vec2::ZERO, pitch_mm);
    grid.origin = grid.to_grid(origin_world);
    grid.to_world(grid.center(i, j))
}

fn too_many_cells() -> UbotError {
    UbotError::Limit(format!("the grid needs more than {MAX_CELLS} cells"))
}

/// Integers strictly between `a` and `b`, as an inclusive range (empty when lo > hi).
fn open_span(a: f64, b: f64) -> Result<(i64, i64), UbotError> {
    let ok = |t: f64| t.is_finite() && t.abs() <= MAX_CELL_INDEX;
    if !(ok(a) && ok(b)) {
        return Err(UbotError::Limit("the grid origin is too far from the boundary".into()));
    }
    Ok((a.floor() as i64 + 1, b.ceil() as i64 - 1))
}

/// Like [`open_span`] but clamped to `range`; `None` when nothing is left.
fn clip_span(a: f64, b: f64, range: (i64, i64)) -> Option<(i64, i64)> {
    let lo = (a.floor() + 1.0).max(range.0 as f64);
    let hi = (b.ceil() - 1.0).min(range.1 as f64);
    // NaN fails this comparison too.
    if lo <= hi { Some((lo as i64, hi as i64)) } else { None }
}

fn span_len((lo, hi): (i64, i64)) -> Result<usize, UbotError> {
    if hi < lo {
        return Ok(0);
    }
    let n = hi.checked_sub(lo).and_then(|d| d.checked_add(1)).ok_or_else(too_many_cells)?;
    usize::try_from(n).map_err(|_| too_many_cells())
}

/// Shared step counter so hostile geometry fails with `Limit` instead of running for minutes.
pub(super) struct Budget {
    left: u64,
}

impl Budget {
    pub fn new() -> Self {
        Budget { left: MAX_WORK }
    }

    pub fn remaining(&self) -> usize {
        usize::try_from(self.left).unwrap_or(usize::MAX)
    }

    pub fn spend(&mut self, n: usize) -> Result<(), UbotError> {
        let n = u64::try_from(n).unwrap_or(u64::MAX);
        self.left = self.left.checked_sub(n).ok_or_else(|| UbotError::Limit("geometry is too complex to classify".into()))?;
        Ok(())
    }
}

struct ScanEdge {
    ring: u32,
    a: Vec2,
    b: Vec2,
    y_lo: f64,
    y_hi: f64,
}

/// Sweeps a horizontal line downwards over ring edges; each stop lists where the line crosses each ring. Only edges
/// spanning the line are visited, so a row costs its crossings rather than every edge.
pub(super) struct Scanline {
    edges: Vec<ScanEdge>,
    next: usize,
    active: Vec<usize>,
    last_y: f64,
    crossings: Vec<(u32, f64)>,
}

impl Scanline {
    pub fn new(edges: impl IntoIterator<Item = (u32, Vec2, Vec2)>) -> Self {
        let mut edges: Vec<ScanEdge> =
            edges.into_iter().map(|(ring, a, b)| ScanEdge { ring, a, b, y_lo: a.y.min(b.y), y_hi: a.y.max(b.y) }).collect();
        edges.sort_by(|p, q| q.y_hi.total_cmp(&p.y_hi));
        Scanline { edges, next: 0, active: Vec::new(), last_y: f64::INFINITY, crossings: Vec::new() }
    }

    /// Crossings of the line at `y`, sorted by ring then x. Uses the half-open rule of `point_in_polygon`, so the
    /// parity of crossings right of a point matches it. Stops should come with non-increasing `y`.
    pub fn at(&mut self, y: f64, budget: &mut Budget) -> Result<&[(u32, f64)], UbotError> {
        if y > self.last_y || y.is_nan() {
            self.next = 0;
            self.active.clear();
        }
        self.last_y = y;
        while let Some(e) = self.edges.get(self.next)
            && e.y_hi > y
        {
            self.active.push(self.next);
            self.next += 1;
        }
        let edges = &self.edges;
        self.active.retain(|&k| edges.get(k).is_some_and(|e| e.y_lo <= y));
        budget.spend(self.active.len() + 1)?;
        self.crossings.clear();
        for e in self.active.iter().filter_map(|&k| edges.get(k)) {
            let x = e.a.x + (y - e.a.y) * (e.b.x - e.a.x) / (e.b.y - e.a.y);
            self.crossings.push((e.ring, x));
        }
        self.crossings.sort_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)));
        Ok(&self.crossings)
    }
}

/// True when `x` on the scanned line lies inside `ring`: an odd number of its crossings are right of `x`.
pub(super) fn inside(crossings: &[(u32, f64)], ring: u32, x: f64) -> bool {
    let lo = crossings.partition_point(|c| c.0 < ring);
    let hi = crossings.partition_point(|c| c.0 <= ring);
    let own = crossings.get(lo..hi).unwrap_or(&[]);
    (own.len() - own.partition_point(|c| c.1 <= x)) % 2 == 1
}

/// Rings covering a point as it moves right along one scanned row.
pub(super) struct RowCover {
    intervals: Vec<(f64, f64, u32)>,
    next: usize,
    active: Vec<(f64, u32)>,
}

impl RowCover {
    /// `crossings` as returned by [`Scanline::at`]; consecutive pairs per ring bound its inside intervals `[x0, x1)`.
    pub fn new(crossings: &[(u32, f64)]) -> Self {
        let mut intervals = Vec::with_capacity(crossings.len() / 2);
        let mut start = 0;
        while start < crossings.len() {
            let ring = crossings.get(start).map_or(0, |c| c.0);
            let end = start + crossings.get(start..).map_or(0, |s| s.partition_point(|c| c.0 == ring));
            if let Some(own) = crossings.get(start..end) {
                for [p, q] in own.as_chunks::<2>().0 {
                    intervals.push((p.1, q.1, ring));
                }
            }
            start = end.max(start + 1);
        }
        intervals.sort_by(|p, q| p.0.total_cmp(&q.0));
        RowCover { intervals, next: 0, active: Vec::new() }
    }

    /// Rings whose inside contains `x`, sorted; calls must come with increasing `x`.
    pub fn at(&mut self, x: f64, out: &mut Vec<u32>, budget: &mut Budget) -> Result<(), UbotError> {
        while let Some(&(start, end, ring)) = self.intervals.get(self.next)
            && start <= x
        {
            self.active.push((end, ring));
            self.next += 1;
        }
        self.active.retain(|&(end, _)| end > x);
        budget.spend(self.active.len() + 1)?;
        out.clear();
        out.extend(self.active.iter().map(|&(_, ring)| ring));
        out.sort_unstable();
        out.dedup();
        Ok(())
    }
}

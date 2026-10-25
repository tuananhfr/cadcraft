use cadcraft_geom::{Bounds2, Vec2};

use super::ring::Ring;

const MAX_BUCKETS: usize = 1_000_000;
const MAX_ENTRIES: usize = 32_000_000;

struct IndexedEdge {
    ring: u32,
    edge: u32,
    bounds: Bounds2,
}

/// Uniform bucket grid over the edges of several rings; queries return candidate edges near a box.
pub struct EdgeIndex {
    origin: Vec2,
    cell: f64,
    nx: usize,
    ny: usize,
    edges: Vec<IndexedEdge>,
    starts: Vec<usize>,
    entries: Vec<u32>,
}

impl EdgeIndex {
    pub fn new(rings: &[&Ring], cell_mm: f64) -> Self {
        let mut edges = Vec::new();
        let mut segs: Vec<(Vec2, Vec2)> = Vec::new();
        for (ri, ring) in rings.iter().enumerate() {
            for ei in 0..ring.edge_count() {
                let Some((a, b)) = ring.edge(ei) else { continue };
                if !(a.is_finite() && b.is_finite()) {
                    continue;
                }
                let (Ok(ring_id), Ok(edge_id)) = (u32::try_from(ri), u32::try_from(ei)) else { continue };
                edges.push(IndexedEdge { ring: ring_id, edge: edge_id, bounds: Bounds2::from_points([a, b]) });
                segs.push((a, b));
            }
        }
        let total = edges.iter().fold(Bounds2::EMPTY, |acc, e| acc.union(&e.bounds));
        let mut index = EdgeIndex { origin: Vec2::ZERO, cell: 1.0, nx: 0, ny: 0, edges, starts: vec![0], entries: Vec::new() };
        if total.is_empty() || !(total.width().is_finite() && total.height().is_finite()) {
            index.edges.clear();
            return index;
        }
        index.origin = total.min;
        let extent = total.width().max(total.height());
        // Floor keeps one edge from spanning more than ~1000 cells per axis.
        let mut cell = if cell_mm.is_finite() && cell_mm > 0.0 { cell_mm } else { extent / 64.0 };
        cell = cell.max(extent / 1000.0).max(1e-9);
        while !index.build(&segs, &total, cell) {
            cell *= 2.0;
        }
        index
    }

    /// Returns false when the grid would hold too many entries and the cell size must grow.
    fn build(&mut self, segs: &[(Vec2, Vec2)], total: &Bounds2, mut cell: f64) -> bool {
        let (mut nx, mut ny) = (grid_len(total.width(), cell), grid_len(total.height(), cell));
        while nx.saturating_mul(ny) > MAX_BUCKETS {
            cell *= 2.0;
            nx = grid_len(total.width(), cell);
            ny = grid_len(total.height(), cell);
        }
        self.cell = cell;
        self.nx = nx;
        self.ny = ny;
        let mut counts = vec![0usize; nx * ny + 1];
        let mut entries_total = 0usize;
        for (a, b) in segs {
            self.for_cells(*a, *b, |c| {
                if let Some(slot) = counts.get_mut(c + 1) {
                    *slot += 1;
                }
                entries_total += 1;
            });
            if entries_total > MAX_ENTRIES {
                return false;
            }
        }
        for i in 1..counts.len() {
            let prev = counts.get(i - 1).copied().unwrap_or(0);
            if let Some(c) = counts.get_mut(i) {
                *c += prev;
            }
        }
        let mut fill = counts.clone();
        let mut entries = vec![0u32; entries_total];
        for (id, (a, b)) in segs.iter().enumerate() {
            let Ok(id) = u32::try_from(id) else { continue };
            self.for_cells(*a, *b, |c| {
                if let Some(pos) = fill.get_mut(c)
                    && let Some(slot) = entries.get_mut(*pos)
                {
                    *slot = id;
                    *pos += 1;
                }
            });
        }
        self.starts = counts;
        self.entries = entries;
        true
    }

    fn cell_x(&self, x: f64) -> usize {
        clamp_cell((x - self.origin.x) / self.cell, self.nx)
    }

    fn cell_y(&self, y: f64) -> usize {
        clamp_cell((y - self.origin.y) / self.cell, self.ny)
    }

    /// Visits every cell the segment passes through, column by column.
    fn for_cells(&self, a: Vec2, b: Vec2, mut f: impl FnMut(usize)) {
        let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
        let (c0, c1) = (self.cell_x(min_x), self.cell_x(max_x));
        let dx = b.x - a.x;
        let pad = self.cell * 1e-6;
        for cx in c0..=c1 {
            let lo = (self.origin.x + cx as f64 * self.cell).max(min_x);
            let hi = (self.origin.x + (cx + 1) as f64 * self.cell).min(max_x);
            let (y0, y1) = if dx.abs() <= f64::EPSILON * max_x.abs().max(min_x.abs()).max(1.0) {
                (a.y.min(b.y), a.y.max(b.y))
            } else {
                let y_at = |x: f64| a.y + (x - a.x) * (b.y - a.y) / dx;
                let (p, q) = (y_at(lo), y_at(hi));
                (p.min(q), p.max(q))
            };
            let (r0, r1) = (self.cell_y(y0 - pad), self.cell_y(y1 + pad));
            for cy in r0..=r1 {
                f(cy * self.nx + cx);
            }
        }
    }

    /// Edges `(ring_idx, edge_idx)` whose bounds meet `bounds` grown by `margin_mm`; sorted, no repeats.
    pub fn query(&self, bounds: Bounds2, margin_mm: f64) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.collect(bounds, margin_mm).into_iter()
    }

    fn collect(&self, bounds: Bounds2, margin_mm: f64) -> Vec<(usize, usize)> {
        let finite = bounds.min.is_finite() && bounds.max.is_finite() && margin_mm.is_finite();
        if !finite || bounds.is_empty() || self.nx == 0 || self.ny == 0 {
            return Vec::new();
        }
        let q = bounds.expand(margin_mm.max(0.0));
        let grid_max = Vec2::new(self.origin.x + self.nx as f64 * self.cell, self.origin.y + self.ny as f64 * self.cell);
        let outside = q.max.x < self.origin.x || q.max.y < self.origin.y || q.min.x > grid_max.x || q.min.y > grid_max.y;
        if !(q.min.is_finite() && q.max.is_finite()) || outside {
            return Vec::new();
        }
        let (x0, x1, y0, y1) = (self.cell_x(q.min.x), self.cell_x(q.max.x), self.cell_y(q.min.y), self.cell_y(q.max.y));
        let mut ids: Vec<u32> = Vec::new();
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                let c = cy * self.nx + cx;
                let (Some(&s), Some(&e)) = (self.starts.get(c), self.starts.get(c + 1)) else { continue };
                if let Some(slice) = self.entries.get(s..e) {
                    ids.extend_from_slice(slice);
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .filter_map(|id| self.edges.get(id as usize))
            .filter(|e| e.bounds.intersects(&q))
            .map(|e| (e.ring as usize, e.edge as usize))
            .collect()
    }
}

fn grid_len(extent: f64, cell: f64) -> usize {
    let n = (extent / cell).floor() + 1.0;
    if n.is_finite() && n >= 1.0 { n.min(1e9) as usize } else { 1 }
}

fn clamp_cell(v: f64, n: usize) -> usize {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    v.min(n.saturating_sub(1) as f64) as usize
}

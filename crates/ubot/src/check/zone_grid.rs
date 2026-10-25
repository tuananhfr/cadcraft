//! Bucket grid over zone bounds: a box only visits the zones near it instead of all of them.

use std::collections::HashMap;

use cadcraft_geom::Bounds2;

/// A zone spanning more buckets than this is kept in a short list every query scans.
const MAX_BUCKETS_PER_ZONE: i128 = 16;
/// A query spanning more buckets than this just returns every zone.
const MAX_QUERY_BUCKETS: i128 = 10_000;

pub(super) struct ZoneGrid {
    cell: f64,
    buckets: HashMap<(i64, i64), Vec<u32>>,
    large: Vec<u32>,
    count: usize,
}

impl ZoneGrid {
    /// `bounds` are the zone extents, already grown by the widest reach a query needs.
    pub fn new(bounds: &[Bounds2]) -> Self {
        let mut extents: Vec<f64> = bounds.iter().map(|b| b.width().max(b.height())).filter(|e| e.is_finite()).collect();
        extents.sort_unstable_by(f64::total_cmp);
        // Twice the median extent: a typical zone lands in at most 4 buckets.
        let cell = extents.get(extents.len() / 2).map_or(1.0, |m| (m * 2.0).max(1e-3));
        let mut grid = ZoneGrid { cell, buckets: HashMap::new(), large: Vec::new(), count: bounds.len() };
        for (k, b) in bounds.iter().enumerate() {
            let Ok(id) = u32::try_from(k) else { continue };
            match grid.range(b) {
                Some((x0, x1, y0, y1)) if span(x0, x1, y0, y1) <= MAX_BUCKETS_PER_ZONE => {
                    for cx in x0..=x1 {
                        for cy in y0..=y1 {
                            grid.buckets.entry((cx, cy)).or_default().push(id);
                        }
                    }
                }
                _ => grid.large.push(id),
            }
        }
        grid
    }

    fn range(&self, b: &Bounds2) -> Option<(i64, i64, i64, i64)> {
        if b.is_empty() || !(b.min.is_finite() && b.max.is_finite()) {
            return None;
        }
        let bucket = |v: f64| (v / self.cell).floor() as i64;
        Some((bucket(b.min.x), bucket(b.max.x), bucket(b.min.y), bucket(b.max.y)))
    }

    /// Ids of zones whose bounds may meet `query`, ascending and without repeats.
    pub fn candidates(&self, query: &Bounds2) -> Vec<usize> {
        let mut ids: Vec<u32> = self.large.clone();
        match self.range(query) {
            Some((x0, x1, y0, y1)) if span(x0, x1, y0, y1) <= MAX_QUERY_BUCKETS => {
                for cx in x0..=x1 {
                    for cy in y0..=y1 {
                        if let Some(b) = self.buckets.get(&(cx, cy)) {
                            ids.extend_from_slice(b);
                        }
                    }
                }
            }
            _ => return (0..self.count).collect(),
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().map(|i| i as usize).collect()
    }
}

fn span(x0: i64, x1: i64, y0: i64, y1: i64) -> i128 {
    (i128::from(x1) - i128::from(x0) + 1) * (i128::from(y1) - i128::from(y0) + 1)
}

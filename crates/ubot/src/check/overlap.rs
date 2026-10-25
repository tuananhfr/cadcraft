//! Box-against-box overlap by bucketing footprint bounds, so 100 000 boxes cost near-linear work instead of n².

use cadcraft_geom::Bounds2;

use super::work::Work;
use crate::error::UbotError;
use crate::fill::footprints_overlap;
use crate::geometry::Footprint;

const MAX_ENTRIES: usize = 4_000_000;
const MAX_PAIRS: usize = 100_000;

/// Index pairs `(a, b)`, `a < b`, of footprints that overlap with positive area; touching is not overlap.
pub(super) fn overlapping_pairs(fps: &[Footprint], work: &mut Work) -> Result<Vec<(usize, usize)>, UbotError> {
    let boxes: Vec<Bounds2> = fps.iter().map(|f| Bounds2::from_points(f.corners)).collect();
    let mut extents: Vec<f64> = boxes.iter().map(|b| b.width().max(b.height())).collect();
    extents.sort_unstable_by(f64::total_cmp);
    // Twice the median extent: a typical box touches at most 4 buckets.
    let cell = extents.get(extents.len() / 2).map_or(1.0, |m| (m * 2.0).max(1e-3));
    if !cell.is_finite() {
        return Err(UbotError::BadInput("box size is not a finite number".into()));
    }
    let bucket = |v: f64| (v / cell).floor() as i64;

    let mut entries: Vec<((i64, i64), u32)> = Vec::new();
    for (k, b) in boxes.iter().enumerate() {
        let id = u32::try_from(k).map_err(|_| UbotError::Limit("too many boxes".into()))?;
        let (x0, x1, y0, y1) = (bucket(b.min.x), bucket(b.max.x), bucket(b.min.y), bucket(b.max.y));
        let count = (i128::from(x1) - i128::from(x0) + 1) * (i128::from(y1) - i128::from(y0) + 1);
        if count < 1 || count + entries.len() as i128 > MAX_ENTRIES as i128 {
            return Err(UbotError::Limit("boxes cover too many overlap buckets".into()));
        }
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                entries.push(((cx, cy), id));
            }
        }
    }
    entries.sort_unstable();

    let mut pairs = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let key = entries.get(start).map(|e| e.0);
        let len = entries.get(start..).map_or(0, |rest| rest.iter().take_while(|e| Some(e.0) == key).count());
        let group = entries.get(start..start + len).unwrap_or(&[]);
        start += len.max(1);
        work.spend(len.saturating_mul(len.saturating_sub(1)) / 2)?;
        for (n, &(_, a)) in group.iter().enumerate() {
            for &(_, b) in group.iter().skip(n + 1) {
                let (ia, ib) = (a as usize, b as usize);
                let (Some(ba), Some(bb)) = (boxes.get(ia), boxes.get(ib)) else { continue };
                if !ba.intersects(bb) {
                    continue;
                }
                // Both boxes share every bucket their bounds meet in; report the pair once, in the bucket of the shared corner.
                if Some((bucket(ba.min.x.max(bb.min.x)), bucket(ba.min.y.max(bb.min.y)))) != key {
                    continue;
                }
                let (Some(fa), Some(fb)) = (fps.get(ia), fps.get(ib)) else { continue };
                if footprints_overlap(fa, fb) {
                    if pairs.len() >= MAX_PAIRS {
                        return Err(UbotError::Limit(format!("more than {MAX_PAIRS} overlapping pairs")));
                    }
                    pairs.push((ia, ib));
                }
            }
        }
    }
    Ok(pairs)
}

//! Quantity take-off (UBCOUNT): tagged and legacy boxes counted per product, unknown blocks per name.

use std::collections::BTreeMap;

use cadcraft_geom::Vec2;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::error::UbotError;
use crate::fill::NEAR_BAND_MM;
use crate::geometry::{RingInput, point_in_ring, ring_from_input};

/// Items one count takes.
pub const MAX_COUNT_ITEMS: usize = 1_000_000;
/// Items × boundary edges tested for the inside check.
const MAX_INSIDE_WORK: usize = 400_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CountSource {
    Tagged,
    Legacy,
}

/// One block reference; `multiplier` is how many boxes it stands for (nested inserts, MINSERT grids).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountItem {
    #[serde(default)]
    pub product_id: Option<String>,
    pub block_name: String,
    pub multiplier: u64,
    pub source: CountSource,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CountRow {
    #[serde(default)]
    pub product_id: Option<String>,
    pub name: String,
    pub tagged: u64,
    pub legacy: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    /// Catalog products in catalog order, then unknown blocks by name; rows with no boxes are left out.
    pub rows: Vec<CountRow>,
    pub total: u64,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    tagged: u64,
    legacy: u64,
}

impl Tally {
    fn add(&mut self, item: &CountItem) {
        let slot = match item.source {
            CountSource::Tagged => &mut self.tagged,
            CountSource::Legacy => &mut self.legacy,
        };
        *slot = slot.saturating_add(item.multiplier);
    }

    fn total(self) -> u64 {
        self.tagged.saturating_add(self.legacy)
    }
}

pub fn count(items: &[CountItem], catalog: &Catalog) -> Counts {
    tally(items.iter(), catalog)
}

fn tally<'a>(items: impl Iterator<Item = &'a CountItem>, catalog: &Catalog) -> Counts {
    let mut known = vec![Tally::default(); catalog.products.len()];
    // Keyed by lower-case name; the spelling shown is the smallest one seen, so input order cannot change the output.
    let mut unknown: BTreeMap<String, (String, Tally)> = BTreeMap::new();
    for item in items {
        let known_slot = match item.product_id.as_deref() {
            Some(id) => catalog.products.iter().position(|p| p.id == id),
            None => catalog
                .mapping_for_block(&item.block_name)
                .and_then(|m| m.product_id.as_deref())
                .and_then(|id| catalog.products.iter().position(|p| p.id == id)),
        };
        match known_slot.and_then(|k| known.get_mut(k)) {
            Some(t) => t.add(item),
            None => {
                let entry = unknown.entry(item.block_name.to_lowercase()).or_insert_with(|| (item.block_name.clone(), Tally::default()));
                if item.block_name < entry.0 {
                    entry.0 = item.block_name.clone();
                }
                entry.1.add(item);
            }
        }
    }
    let mut rows: Vec<CountRow> = catalog
        .products
        .iter()
        .zip(known)
        .filter(|(_, t)| t.total() > 0)
        .map(|(p, t)| CountRow { product_id: Some(p.id.clone()), name: p.name.clone(), tagged: t.tagged, legacy: t.legacy, total: t.total() })
        .collect();
    rows.extend(unknown.into_values().filter(|(_, t)| t.total() > 0).map(|(name, t)| CountRow {
        product_id: None,
        name,
        tagged: t.tagged,
        legacy: t.legacy,
        total: t.total(),
    }));
    let total = rows.iter().fold(0u64, |n, r| n.saturating_add(r.total));
    Counts { rows, total }
}

/// Counts the items whose centre (drawing units) lies inside `boundary`; `None` counts all. `boundary` carries the unit scale.
pub fn count_in(items: &[(CountItem, [f64; 2])], boundary: Option<(&RingInput, f64)>, catalog: &Catalog) -> Result<Counts, UbotError> {
    if items.len() > MAX_COUNT_ITEMS {
        return Err(UbotError::Limit(format!("{} items, the limit is {MAX_COUNT_ITEMS}", items.len())));
    }
    let Some((ring, unit_scale)) = boundary else {
        return Ok(tally(items.iter().map(|i| &i.0), catalog));
    };
    let ring = ring_from_input(ring, unit_scale, NEAR_BAND_MM)?;
    if items.len().saturating_mul(ring.edge_count()) > MAX_INSIDE_WORK {
        return Err(UbotError::Limit("too many items for this boundary".into()));
    }
    let inside = |c: &[f64; 2]| {
        let p = Vec2::new(c[0] * unit_scale, c[1] * unit_scale);
        p.is_finite() && point_in_ring(p, &ring)
    };
    Ok(tally(items.iter().filter(|(_, c)| inside(c)).map(|i| &i.0), catalog))
}

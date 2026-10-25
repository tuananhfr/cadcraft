use crate::error::UbotError;
use crate::model::{Anchor, BlockMapping, Product, RulePack};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "ubot.catalog/1";
const MAX_ENTRIES: usize = 10_000;
const BUILTIN_JSON: &str = include_str!("catalog_builtin.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub schema: String,
    pub catalog_id: String,
    pub version: String,
    pub products: Vec<Product>,
    pub mappings: Vec<BlockMapping>,
    pub rulepacks: Vec<RulePack>,
}

impl Catalog {
    /// Embedded draft catalog; falls back to an empty valid catalog if the embedded JSON is broken.
    pub fn builtin() -> Catalog {
        Self::from_json(BUILTIN_JSON).unwrap_or_else(|_| Catalog {
            schema: SCHEMA.to_string(),
            catalog_id: "lpc-builtin".to_string(),
            version: "draft-1".to_string(),
            products: Vec::new(),
            mappings: Vec::new(),
            rulepacks: Vec::new(),
        })
    }

    pub fn from_json(s: &str) -> Result<Catalog, UbotError> {
        let c: Catalog = serde_json::from_str(s).map_err(|e| UbotError::BadInput(format!("catalog json: {e}")))?;
        c.validate()?;
        Ok(c)
    }

    fn validate(&self) -> Result<(), UbotError> {
        if self.schema != SCHEMA {
            return Err(UbotError::BadInput(format!("unsupported catalog schema '{}'", self.schema)));
        }
        for (what, len) in [("products", self.products.len()), ("mappings", self.mappings.len()), ("rulepacks", self.rulepacks.len())] {
            if len > MAX_ENTRIES {
                return Err(UbotError::BadInput(format!("too many {what} (max {MAX_ENTRIES})")));
            }
        }
        let mut seen = std::collections::HashSet::new();
        for p in &self.products {
            if !seen.insert(p.id.as_str()) {
                return Err(UbotError::BadInput(format!("duplicate product id '{}'", p.id)));
            }
            positive(p.width_mm, "widthMm")?;
            positive(p.length_mm, "lengthMm")?;
            for (v, n) in [(p.height_mm, "heightMm"), (p.slab_thickness_mm, "slabThicknessMm")] {
                if let Some(v) = v {
                    positive(v, n)?;
                }
            }
        }
        for m in &self.mappings {
            positive(m.units_mm_per_block_unit, "unitsMmPerBlockUnit")?;
            finite(m.rotation_offset_deg, "rotationOffsetDeg")?;
            if let Anchor::Offset { x_mm, y_mm } = m.anchor {
                finite(x_mm, "anchor.xMm")?;
                finite(y_mm, "anchor.yMm")?;
            }
        }
        for r in &self.rulepacks {
            positive(r.rib_mm_default, "ribMmDefault")?;
            for &c in &r.rib_mm_choices {
                positive(c, "ribMmChoices")?;
            }
            for (v, n) in [(r.edge_clearance_mm, "edgeClearanceMm"), (r.exclusion_clearance_mm, "exclusionClearanceMm")] {
                finite(v, n)?;
                if v < 0.0 {
                    return Err(UbotError::BadInput(format!("{n} must be >= 0")));
                }
            }
        }
        Ok(())
    }

    /// Entries of `other` replace same-id entries in place; new ones are appended.
    pub fn overlay(&self, other: &Catalog) -> Catalog {
        Catalog {
            schema: self.schema.clone(),
            catalog_id: self.catalog_id.clone(),
            version: self.version.clone(),
            products: merge(&self.products, &other.products, |p| p.id.clone()),
            mappings: merge(&self.mappings, &other.mappings, |m| m.block_name.to_lowercase()),
            rulepacks: merge(&self.rulepacks, &other.rulepacks, |r| r.id.clone()),
        }
    }

    pub fn product(&self, id: &str) -> Option<&Product> {
        self.products.iter().find(|p| p.id == id)
    }

    /// `None` selects the first rulepack.
    pub fn rulepack(&self, id: Option<&str>) -> Option<&RulePack> {
        match id {
            Some(id) => self.rulepacks.iter().find(|r| r.id == id),
            None => self.rulepacks.first(),
        }
    }

    pub fn mapping_for_block(&self, name: &str) -> Option<&BlockMapping> {
        self.mappings.iter().find(|m| m.block_name.eq_ignore_ascii_case(name))
    }

    pub fn mapping_for_product(&self, product_id: &str) -> Option<&BlockMapping> {
        self.mappings.iter().find(|m| m.product_id.as_deref() == Some(product_id))
    }

    /// `UBOT_<SKU>` with every character outside `[A-Z0-9]` replaced by `_`.
    pub fn default_block_name(product: &Product) -> String {
        let sku: String = product.sku.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
        format!("UBOT_{sku}")
    }
}

fn merge<T: Clone>(base: &[T], other: &[T], key: impl Fn(&T) -> String) -> Vec<T> {
    let mut out: Vec<T> = base.to_vec();
    for item in other {
        let k = key(item);
        match out.iter().position(|e| key(e) == k) {
            Some(i) => {
                if let Some(slot) = out.get_mut(i) {
                    *slot = item.clone();
                }
            }
            None => out.push(item.clone()),
        }
    }
    out
}

fn finite(v: f64, name: &str) -> Result<(), UbotError> {
    if v.is_finite() { Ok(()) } else { Err(UbotError::BadInput(format!("{name} must be finite"))) }
}

fn positive(v: f64, name: &str) -> Result<(), UbotError> {
    finite(v, name)?;
    if v > 0.0 { Ok(()) } else { Err(UbotError::BadInput(format!("{name} must be > 0"))) }
}

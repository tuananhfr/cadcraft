//! Which fonts the browser keeps for the next visit, and how much room they may take. localStorage
//! holds a few MB for the whole site, so a font over [`MAX_FONT_BYTES`], or one that would take the
//! fonts past [`MAX_TOTAL_BYTES`], works for this visit only.

use serde_json::{Value, json};

pub const MAX_FONT_BYTES: usize = 3 << 20;
pub const MAX_TOTAL_BYTES: usize = 4 << 20;

/// The most fonts the index lists (it is read from storage the page doesn't control).
const MAX_ENTRIES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The file name the font was added with.
    pub name: String,
    /// What the engine calls it (`cadcraft_fonts::ttf::key` of the name).
    pub stem: String,
    pub size: usize,
}

#[derive(Default)]
pub struct Index {
    entries: Vec<Entry>,
}

impl Index {
    /// Read a saved index; anything unreadable is dropped.
    pub fn parse(json: &str) -> Index {
        let list = serde_json::from_str::<Value>(json).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let entries = list
            .iter()
            .filter_map(|e| {
                let name = e["name"].as_str().filter(|n| !n.is_empty())?.to_string();
                let stem = e["stem"].as_str().filter(|s| !s.is_empty())?.to_string();
                Some(Entry { name, stem, size: usize::try_from(e["size"].as_u64()?).ok()? })
            })
            .take(MAX_ENTRIES)
            .collect();
        Index { entries }
    }

    pub fn to_json(&self) -> String {
        Value::Array(self.entries.iter().map(|e| json!({"name": e.name, "stem": e.stem, "size": e.size})).collect()).to_string()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Why a font called `stem` of `size` bytes can't be kept, if it can't. Adding a font again
    /// replaces the old one, so its size doesn't count.
    pub fn refusal(&self, stem: &str, size: usize) -> Option<String> {
        if size > MAX_FONT_BYTES {
            return Some(format!("This font is {} MB; the browser keeps fonts up to {} MB.", size.div_ceil(1 << 20), MAX_FONT_BYTES >> 20));
        }
        let others: usize = self.entries.iter().filter(|e| e.stem != stem).map(|e| e.size).sum();
        if self.entries.len() >= MAX_ENTRIES || others + size > MAX_TOTAL_BYTES {
            return Some(format!("The browser keeps up to {} MB of fonts, and that is full.", MAX_TOTAL_BYTES >> 20));
        }
        None
    }

    /// Record a font, replacing one with the same stem.
    pub fn insert(&mut self, entry: Entry) {
        self.entries.retain(|e| e.stem != entry.stem);
        self.entries.push(entry);
    }

    pub fn remove(&mut self, stem: &str) -> Option<Entry> {
        let at = self.entries.iter().position(|e| e.stem == stem)?;
        Some(self.entries.remove(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(stem: &str, size: usize) -> Entry {
        Entry { name: format!("{stem}.ttf"), stem: stem.to_string(), size }
    }

    #[test]
    fn the_index_survives_storage_and_ignores_junk() {
        let mut i = Index::default();
        i.insert(entry("a", 10));
        i.insert(entry("b", 20));
        assert_eq!(Index::parse(&i.to_json()).entries(), i.entries());
        assert!(Index::parse("not json").entries().is_empty());
        assert_eq!(Index::parse(r#"[{"name":"x.ttf","stem":"x","size":5},{"name":3},{"stem":"y"}]"#).entries().len(), 1);
    }

    #[test]
    fn a_font_over_the_caps_is_refused_but_a_replacement_is_not_counted_twice() {
        let mut i = Index::default();
        assert!(i.refusal("big", MAX_FONT_BYTES + 1).is_some());
        i.insert(entry("a", MAX_FONT_BYTES));
        assert_eq!(i.refusal("a", MAX_FONT_BYTES), None, "replacing a with the same size fits");
        assert_eq!(i.refusal("b", MAX_TOTAL_BYTES - MAX_FONT_BYTES), None);
        assert!(i.refusal("b", MAX_TOTAL_BYTES - MAX_FONT_BYTES + 1).is_some());
        assert_eq!(i.remove("a").map(|e| e.size), Some(MAX_FONT_BYTES));
        assert!(i.remove("a").is_none());
    }
}

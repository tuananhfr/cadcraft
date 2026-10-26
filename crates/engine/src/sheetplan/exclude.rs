use std::borrow::Cow;

use cadcraft_doc::{Drawing, Layer};

use super::Sheet;

/// The drawing these sheets plot from: their excluded layers are made non-plotting in a copy.
/// One PDF is one plot call, so a merged file applies the union of its sheets' exclusions.
pub(crate) fn plot_drawing<'a>(d: &'a Drawing, sheets: &[Sheet]) -> Cow<'a, Drawing> {
    let mut names: Vec<&str> = Vec::new();
    for name in sheets.iter().flat_map(|sheet| &sheet.exclude_layers) {
        if !names.iter().any(|known| known.eq_ignore_ascii_case(name)) {
            names.push(name);
        }
    }
    if names.is_empty() {
        return Cow::Borrowed(d);
    }
    let mut copy = d.clone();
    for name in names {
        match copy.layer_mut(name) {
            Some(layer) => layer.plot = false,
            None => copy.layers.push(Layer { plot: false, ..Layer::new(name) }),
        }
    }
    Cow::Owned(copy)
}

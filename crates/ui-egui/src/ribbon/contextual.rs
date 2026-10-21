//! Selection contexts shared by every ribbon preset.

use cadcraft_engine::Session;
use cadcraft_engine::doc::EntityKind;

pub fn active_contexts(session: &Session) -> Vec<&'static str> {
    let Ok(state) = session.state() else { return Vec::new() };
    if state.selection.is_empty() {
        return Vec::new();
    }
    for (context, dimension) in [("dimension", true), ("hatch", false)] {
        if state.selection.iter().all(|handle| {
            state.doc.entity(*handle).is_some_and(|entity| {
                if dimension { matches!(entity.kind, EntityKind::Dimension(_)) } else { matches!(entity.kind, EntityKind::Hatch(_)) }
            })
        }) {
            return vec![context];
        }
    }
    Vec::new()
}

use serde_json::json;

use crate::CadApp;

pub const DIALOG: &str = "workspace_tab_conflict";

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let mut keep = None;
    egui::Window::new(crate::tl!("Changed in another tab")).id(egui::Id::new(DIALOG)).open(open).collapsible(false).resizable(false).show(
        ctx,
        |ui| {
            ui.horizontal(|ui| {
                if ui.button(crate::tl!("Keep my layout")).clicked() {
                    keep = Some("mine");
                }
                if ui.button(crate::tl!("Use the other tab's layout")).clicked() {
                    keep = Some("theirs");
                }
            });
        },
    );
    if let Some(keep) = keep {
        match app.run("ui.workspace.resolve", json!({"keep": keep})) {
            Ok(_) => *open = false,
            Err(error) => app.set_status(error),
        }
    }
}

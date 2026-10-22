use serde_json::{Value, json};

use crate::CadApp;

pub const DIALOG: &str = "workspace_import_preview";
const MAX_ROWS: usize = 200;

pub(crate) struct Preview {
    text: String,
    changes: Vec<String>,
    ignored: Vec<String>,
    warnings: Vec<String>,
}

impl CadApp {
    pub fn open_import_preview(&mut self, text: String) {
        let _ = open(self, text);
    }
}

pub(crate) fn open(app: &mut CadApp, text: String) -> Result<Value, String> {
    app.ws.import_preview = None;
    if app.ui.dialog.as_deref() == Some(DIALOG) {
        app.ui.dialog = None;
    }
    let result = match app.run("ui.workspace.import", json!({"text": text, "dry_run": true})) {
        Ok(result) => result,
        Err(error) => {
            let message = crate::i18n::with_language(app.language(), || crate::i18n::t(&error).to_string());
            app.set_status(message);
            return Err(error);
        }
    };
    let strings = |key| result.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    app.ws.import_preview = Some(Preview { text, changes: strings("changes"), ignored: strings("ignored"), warnings: strings("warnings") });
    app.ui.dialog = Some(DIALOG.into());
    Ok(result)
}

pub fn dialog(app: &mut CadApp, ctx: &egui::Context, open: &mut bool) {
    let Some(preview) = app.ws.import_preview.as_ref() else {
        *open = false;
        return;
    };
    let mut confirm = false;
    let mut cancel = false;
    egui::Window::new(crate::tl!("Import Workspace"))
        .id(egui::Id::new(DIALOG))
        .open(open)
        .collapsible(false)
        .default_size(egui::vec2(520.0, 420.0))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                ui.heading(crate::tl!("Changes"));
                rows(ui, &preview.changes, false);
                if !preview.ignored.is_empty() {
                    ui.heading(crate::tl!("Ignored device settings"));
                    rows(ui, &preview.ignored, false);
                }
                if !preview.warnings.is_empty() {
                    ui.heading(crate::tl!("Warnings"));
                    rows(ui, &preview.warnings, true);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                confirm = ui.button(crate::tl!("Import")).clicked();
                cancel = ui.button(crate::tl!("Cancel")).clicked();
            });
        });
    if confirm {
        // Revalidate the retained bytes so confirmation uses the same checks as programmatic imports.
        let text = preview.text.clone();
        match app.run("ui.workspace.import", json!({"text": text})) {
            Ok(_) => *open = false,
            Err(error) => app.set_status(crate::i18n::t(&error)),
        }
    }
    if cancel {
        *open = false;
    }
    if !*open {
        app.ws.import_preview = None;
    }
}

fn rows(ui: &mut egui::Ui, items: &[String], translate: bool) {
    for item in items.iter().take(MAX_ROWS) {
        ui.label(if translate { crate::i18n::t(item) } else { item.as_str() });
    }
    if items.len() > MAX_ROWS {
        ui.label(crate::tf!("{count} more items", count = items.len() - MAX_ROWS));
    }
}

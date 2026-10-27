//! Autosave and Drawing Recovery in the app. The engine decides what to write and writes it
//! ([`cadcraft_engine::autosave`]); this runs it every frame, keeps SAVETIME and SAVEFILEPATH with
//! the preferences, and shows the Drawing Recovery list after a crash: each autosaved drawing can
//! be opened (as an unsaved copy) or discarded. Browser hosts provide asynchronous storage.

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use cadcraft_engine::autosave::{self, Job, Recoverable};
use cadcraft_engine::doc::Drawing;
use egui::{RichText, vec2};
use serde_json::{Value, json};

use crate::CadApp;
use crate::theme::Tokens;

/// The host storage key for [`prefs_json`].
pub const PREFS_KEY: &str = "cadcraft.autosave";

/// SAVETIME and SAVEFILEPATH as JSON for the host's storage.
pub fn prefs_json() -> String {
    json!({ "savetime": autosave::savetime(), "saveFilePath": autosave::savefilepath() }).to_string()
}

/// Restore what [`prefs_json`] saved; malformed values are ignored.
pub fn load_prefs(json: &str) {
    let Ok(v) = serde_json::from_str::<Value>(json) else { return };
    if let Some(m) = v.get("savetime").and_then(Value::as_u64) {
        autosave::set_savetime(u32::try_from(m).unwrap_or(autosave::MAX_SAVETIME));
    }
    if let Some(p) = v.get("saveFilePath").and_then(Value::as_str) {
        autosave::set_savefilepath(p);
    }
}

/// What the GPU error notice says (see [`gpu_notice`]).
#[derive(Clone, Debug, PartialEq)]
pub struct GpuNotice {
    pub fault: crate::gpu::GpuFault,
    /// Drawings autosaved because of it (0: nothing unsaved, or autosave is off).
    pub autosaved: usize,
}

/// Drawing Recovery: autosave files left behind by an app that stopped, listed once at start.
#[derive(Clone, Debug, Default)]
pub struct Recovery {
    pub items: Vec<Recoverable>,
}

pub const MAX_WEB_AUTOSAVE_BYTES: usize = 64 << 20;

pub trait AutosaveSink {
    fn write(&mut self, name: &str, title: &str, bytes: Vec<u8>) -> Result<(), String>;
    fn delete(&mut self, name: &str);
    fn beat(&mut self, instance: &str, now_ms: f64);
    fn fetch(&mut self, name: &str);
    fn take_fetched(&mut self) -> Option<(String, Result<Vec<u8>, String>)>;
    fn last_written(&self, name: &str) -> Option<f64>;
    // Hosts may have less free storage than the hard ceiling.
    fn max_bytes(&self) -> usize {
        MAX_WEB_AUTOSAVE_BYTES
    }
    // Queue acceptance cannot report an eventual transaction abort.
    fn take_failed(&mut self) -> Option<(String, String)> {
        None
    }
}

struct BrowserWrite {
    path: String,
    source: Weak<Drawing>,
    host: Weak<Drawing>,
    previous: Option<f64>,
    retry_at: Option<f64>,
}

pub(crate) struct BrowserAutosave {
    sink: Box<dyn AutosaveSink>,
    waiting: Option<Recoverable>,
    writes: HashMap<u64, BrowserWrite>,
    last_beat: Option<f64>,
    error: Option<String>,
}

impl CadApp {
    pub fn enable_browser_autosave(&mut self, sink: Box<dyn AutosaveSink>, instance: String, leftovers: Vec<Recoverable>) {
        self.autosave = Some(autosave::Autosaver::new(instance));
        self.recovery.items = leftovers;
        self.browser_autosave = Some(BrowserAutosave { sink, waiting: None, writes: HashMap::new(), last_beat: None, error: None });
    }

    pub fn drawing_autosave_error(&self) -> Option<&str> {
        self.browser_autosave.as_ref().and_then(|browser| browser.error.as_deref())
    }

    pub(crate) fn browser_drawing_save(&self) -> Option<crate::wsprofile::save_state::DrawingSave> {
        use crate::wsprofile::save_state::DrawingSave;
        let browser = self.browser_autosave.as_ref()?;
        if browser.error.is_some() {
            return Some(DrawingSave::Dirty);
        }
        let mut at: Option<f64> = None;
        for state in self.session.docs.iter().filter(|state| state.is_dirty()) {
            let Some(write) = browser.writes.get(&state.uid) else { return Some(DrawingSave::Dirty) };
            if !std::ptr::eq(write.source.as_ptr(), Arc::as_ptr(&state.doc))
                || !std::ptr::eq(write.host.as_ptr(), Arc::as_ptr(state.drawing()))
                || write.retry_at.is_some()
            {
                return Some(DrawingSave::Dirty);
            }
            let written = browser.sink.last_written(&write.path);
            if written == write.previous || written.is_none() {
                return Some(DrawingSave::SavingLocal);
            }
            if let Some(written) = written {
                at = Some(at.map_or(written, |old| old.min(written)));
            }
        }
        at.map(|at_ms| DrawingSave::AutosavedLocal { at_ms })
    }

    /// Hosts call this before frame delivery stops; it never deletes the tab's recovery copies.
    pub fn browser_autosave_urgent(&mut self, ctx: &egui::Context) {
        crate::i18n::with_language(self.language(), || self.browser_autosave_tick(ctx, true));
    }

    fn browser_autosave_tick(&mut self, ctx: &egui::Context, urgent: bool) {
        let Some(mut browser) = self.browser_autosave.take() else { return };
        let now = ctx.input(|input| input.time);
        if !now.is_finite() || now < 0.0 {
            self.browser_autosave = Some(browser);
            return;
        }
        if let Some((name, result)) = browser.sink.take_fetched()
            && let Some(item) = browser.waiting.take()
        {
            let recovered = if name != item.path {
                Err(crate::tl!("Could not recover the drawing").into())
            } else {
                result.and_then(|bytes| {
                    if bytes.len() > MAX_WEB_AUTOSAVE_BYTES {
                        return Err(crate::tl!("Drawing too large to autosave in the browser").into());
                    }
                    cadcraft_engine::cmd::audit::recover_bytes(&mut self.session, &bytes, &name, Some(&item.drawing), true)
                        .map_err(|error| error.to_string())
                })
            };
            match recovered {
                Ok(result) => {
                    if let (Some(saver), Some(uid)) = (&mut self.autosave, result.get("uid").and_then(Value::as_u64)) {
                        saver.adopt(&self.session, uid, &item.path);
                    }
                    self.recovery.items.retain(|candidate| candidate.path != item.path);
                    self.ui.start_tab = false;
                    self.canvas.zoom_pending = true;
                }
                Err(error) => self.set_status(error),
            }
        }
        let minutes = autosave::savetime();
        let retry_delay = f64::from(minutes.max(1)) * 60.0;
        for _ in 0..10_000 {
            let Some((name, error)) = browser.sink.take_failed() else { break };
            for write in browser.writes.values_mut().filter(|write| write.path == name) {
                write.retry_at = Some(now + retry_delay);
            }
            browser.error = Some(error.clone());
            self.set_status(error);
        }
        let Some(saver) = &mut self.autosave else {
            self.browser_autosave = Some(browser);
            return;
        };
        let mut jobs = if urgent { saver.plan_urgent(&self.session, now, minutes, "idb") } else { saver.plan(&self.session, now, minutes, "idb") };
        browser.writes.retain(|uid, _| self.session.docs.iter().any(|state| state.uid == *uid && state.is_dirty()));
        if minutes != 0 {
            for (uid, write) in &browser.writes {
                if write.retry_at.is_some_and(|due| urgent || now >= due)
                    && !jobs.iter().any(|job| matches!(job, Job::Write { uid: other, .. } if other == uid))
                    && let Some(state) = self.session.docs.iter().find(|state| state.uid == *uid)
                {
                    jobs.push(Job::Write { uid: *uid, path: write.path.clone(), doc: state.recovery_drawing() });
                }
            }
        }
        if browser.last_beat.is_none_or(|last| now - last >= 60.0) || urgent {
            browser.sink.beat(&saver.instance, now * 1000.0);
            browser.last_beat = Some(now);
        }
        let next_due = saver.next_due(now, minutes);
        for job in jobs {
            match job {
                Job::Delete { path } => browser.sink.delete(&path),
                Job::Write { uid, path, doc } => {
                    let Some(state) = self.session.docs.iter().find(|state| state.uid == uid) else { continue };
                    let mut write = BrowserWrite {
                        path: path.clone(),
                        source: Arc::downgrade(&state.doc),
                        host: Arc::downgrade(state.drawing()),
                        previous: browser.sink.last_written(&path),
                        retry_at: None,
                    };
                    // Recovery keys end in sv$, but the format hook selects output by extension.
                    let result = cadcraft_engine::cmd::file::io()
                        .ok_or_else(|| crate::tl!("Could not autosave the drawing").to_string())
                        .and_then(|hooks| (hooks.write)(&doc, "autosave.dxf"))
                        .and_then(|bytes| {
                            if bytes.len() > browser.sink.max_bytes().min(MAX_WEB_AUTOSAVE_BYTES) {
                                return Err(crate::tl!("Drawing too large to autosave in the browser").into());
                            }
                            browser.sink.write(&path, &state.title, bytes)
                        });
                    if let Err(error) = result {
                        write.retry_at = Some(now + retry_delay);
                        browser.error = Some(error.clone());
                        self.set_status(error);
                    }
                    browser.writes.insert(uid, write);
                }
            }
        }
        if browser
            .writes
            .values()
            .all(|write| write.retry_at.is_none() && browser.sink.last_written(&write.path).is_some_and(|at| Some(at) != write.previous))
        {
            browser.error = None;
        }
        let retry_due = browser.writes.values().filter_map(|write| write.retry_at).map(|at| (at - now).max(0.0)).reduce(f64::min);
        let wait = next_due.into_iter().chain(retry_due).chain([60.0 - (now - browser.last_beat.unwrap_or(now))]).reduce(f64::min);
        if let Some(wait) = wait {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.01)));
        }
        if browser.waiting.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.browser_autosave = Some(browser);
    }

    /// Turn autosave on (desktop). Files go to SAVEFILEPATH, or to `default_dir` when it isn't
    /// set; files a crashed app left there are listed for Drawing Recovery.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_autosave(&mut self, default_dir: &std::path::Path) {
        if autosave::savefilepath().is_empty() {
            autosave::set_savefilepath(&default_dir.to_string_lossy());
        }
        let saver = autosave::Autosaver::new(autosave::instance_id());
        let dir = std::path::PathBuf::from(autosave::savefilepath());
        autosave::prune_locks(&dir);
        self.recovery.items = autosave::recoverable(&dir, &saver.instance);
        self.autosave = Some(saver);
    }

    /// Open recovery item `i` as an unsaved copy. Its autosave file stays until that drawing is
    /// saved or closed, so a second crash doesn't lose it.
    pub fn recover_item(&mut self, i: usize) -> Result<(), String> {
        let item = self.recovery.items.get(i).cloned().ok_or("no such recovery file")?;
        if let Some(browser) = &mut self.browser_autosave {
            if browser.waiting.is_some() {
                return Ok(());
            }
            browser.sink.fetch(&item.path);
            browser.waiting = Some(item);
            return Ok(());
        }
        let r = self.run("recover", json!({ "path": item.path, "copy": true, "title": item.drawing }))?;
        self.recovery.items.remove(i);
        if let (Some(saver), Some(uid)) = (&mut self.autosave, r.get("uid").and_then(Value::as_u64)) {
            saver.adopt(&self.session, uid, &item.path);
        }
        self.ui.start_tab = false;
        self.canvas.zoom_pending = true;
        Ok(())
    }

    /// Delete recovery item `i`'s file.
    pub fn discard_item(&mut self, i: usize) {
        if i < self.recovery.items.len() {
            let _item = self.recovery.items.remove(i);
            if let Some(browser) = &mut self.browser_autosave {
                if browser.waiting.as_ref().is_some_and(|waiting| waiting.path == _item.path) {
                    browser.waiting = None;
                }
                browser.sink.delete(&_item.path);
                return;
            }
            #[cfg(not(target_arch = "wasm32"))]
            autosave::discard(&_item.path);
        }
    }

    /// Every frame: autosave what is due, and wake up when the next one is.
    pub fn autosave_tick(&mut self, _ctx: &egui::Context) {
        if self.browser_autosave.is_some() {
            crate::i18n::with_language(self.language(), || self.browser_autosave_tick(_ctx, false));
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(wait) = self.autosave.as_mut().and_then(|s| s.tick(&self.session)) {
            _ctx.request_repaint_after(wait);
        }
    }

    /// Every frame: after a GPU error or device loss, autosave unsaved drawings at once (the
    /// session may not survive it) and show [`gpu_notice`].
    pub fn gpu_fault_tick(&mut self, ctx: &egui::Context) {
        let Some(fault) = self.gpu_faults.take() else { return };
        #[cfg(not(target_arch = "wasm32"))]
        let saved = if self.browser_autosave.is_some() {
            self.browser_autosave_urgent(ctx);
            0
        } else {
            self.autosave.as_mut().map_or(0, |a| a.save_now(&self.session))
        };
        #[cfg(target_arch = "wasm32")]
        let saved = {
            self.browser_autosave_urgent(ctx);
            0
        };
        let autosaved = self.gpu_notice.as_ref().map_or(0, |n| n.autosaved) + saved;
        self.gpu_notice = Some(GpuNotice { fault, autosaved });
        ctx.request_repaint();
    }

    /// The app closes normally: this app's autosave files are not needed any more.
    pub fn shutdown_autosave(&mut self) {
        if self.browser_autosave.is_some() {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(s) = &mut self.autosave {
            s.shutdown();
        }
    }
}

/// `secs` since 1970 as `YYYY-MM-DD HH:MM` (UTC).
pub fn utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let (h, m) = ((secs % 86_400) / 3600, (secs % 3600) / 60);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}")
}

/// The Drawing Recovery window, while autosave files from a crash are waiting.
pub fn show(app: &mut CadApp, ctx: &egui::Context) {
    if app.recovery.items.is_empty() {
        return;
    }
    let t = Tokens::get();
    let mut open = None;
    let mut discard = None;
    let mut later = false;
    egui::Modal::new(egui::Id::new("cc_drawing_recovery")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(crate::tl!("Drawing Recovery")).strong());
        ui.add_space(4.0);
        ui.label(crate::tl!("UbotCAD did not close normally. These drawings were autosaved:"));
        ui.add_space(6.0);
        egui::Grid::new("cc_recovery_list").num_columns(4).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
            for (i, item) in app.recovery.items.iter().enumerate() {
                ui.label(&item.drawing);
                ui.label(
                    RichText::new(crate::tf!("{date} UTC · {size} KB", date = utc(item.modified), size = item.bytes.div_ceil(1024)))
                        .color(t.text_dim),
                );
                if ui.button(crate::tl!("Open")).clicked() {
                    open = Some(i);
                }
                if ui.button(crate::tl!("Discard")).clicked() {
                    discard = Some(i);
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(crate::tl!("Opened drawings have no file name: save them to keep them.")).color(t.text_dim));
        ui.add_space(8.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button(crate::tl!("Later")).on_hover_text(crate::tl!("Keep the files and ask again next time")).clicked() {
                later = true;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            later = true;
        }
    });
    if let Some(i) = open
        && let Err(e) = app.recover_item(i)
    {
        app.set_status(e);
    }
    if let Some(i) = discard {
        app.discard_item(i);
    }
    if later {
        app.recovery.items.clear();
    }
    if open.is_some() || discard.is_some() || later {
        ctx.request_repaint();
    }
}

/// The notice after a GPU error or device loss: what happened, what was autosaved, and what to
/// do. Not modal, so drawings can still be saved while the device works.
pub fn gpu_notice(app: &mut CadApp, ctx: &egui::Context) {
    let Some(n) = &app.gpu_notice else { return };
    let t = Tokens::get();
    let mut dismiss = false;
    egui::Window::new(crate::tl!("Graphics error"))
        .id(egui::Id::new("cc_gpu_notice"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_TOP, vec2(0.0, 48.0))
        .show(ctx, |ui| {
            ui.set_width(440.0);
            if n.fault.lost {
                ui.label(crate::tl!(
                    "The graphics device stopped working (a driver reset or a GPU that stopped responding). UbotCAD can't draw again until it restarts."
                ));
            } else {
                let times = if n.fault.errors == 1 { String::new() } else { crate::tf!(" ({count} times)", count = n.fault.errors) };
                ui.label(crate::tf!(
                    "The graphics driver reported an error{times}. UbotCAD kept running, but the drawing may not display correctly.",
                    times = times
                ));
            }
            ui.add_space(4.0);
            match n.autosaved {
                0 => ui.label(crate::tl!("Nothing needed autosaving.")),
                1 => ui.label(crate::tl!("A recovery copy of the open drawing with unsaved changes was autosaved.")),
                k => ui.label(crate::tf!("Recovery copies of {count} drawings with unsaved changes were autosaved.", count = k)),
            };
            ui.label(
                RichText::new(crate::tl!("Save your work and restart UbotCAD. Recovery copies are offered at the next start if needed.")).strong(),
            );
            ui.add_space(4.0);
            ui.collapsing(crate::tl!("Details"), |ui| {
                ui.label(RichText::new(&n.fault.message).color(t.text_dim).monospace());
            });
            // A row just as tall as the button (a plain right-to-left layout takes the rest).
            ui.allocate_ui_with_layout(vec2(ui.available_width(), 0.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(crate::tl!("Dismiss")).clicked() {
                    dismiss = true;
                }
            });
        });
    if dismiss {
        app.gpu_notice = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_dates() {
        assert_eq!(super::utc(0), "1970-01-01 00:00");
        assert_eq!(super::utc(951_782_400 + 3_660), "2000-02-29 01:01");
        assert_eq!(super::utc(1_791_676_800), "2026-10-11 00:00");
    }
}

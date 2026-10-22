//! CADCraft in the browser.
//!
//! Runs the same [`cadcraft_ui_egui::CadApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory (output in `dist/web`). URL flags: `?webgl` forces WebGL2, `?cpu` draws the drawing
//! with egui shapes instead of the GPU canvas, `?sample` opens the sample drawing. Saving downloads
//! the drawing through the browser (`download.rs`). With `?host=parent` (inside an `<iframe>`), a
//! same-origin parent page sends the drawing to open and stores what CADCraft saves instead
//! (`host.rs`). The workspace layout is kept in IndexedDB, with a localStorage fallback.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_arch = "wasm32")]
pub mod download;
#[cfg(target_arch = "wasm32")]
mod fullscreen;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(target_arch = "wasm32")]
mod leave_guard;

#[cfg(target_arch = "wasm32")]
mod web {
    use cadcraft_engine::Session;
    use cadcraft_ui_egui::{CadApp, Services};
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    const CANVAS_ID: &str = "cadcraft_canvas";
    const LOADING_ID: &str = "cadcraft_loading";

    /// The app (shared with the page-hide listener, which saves the layout when the tab is left),
    /// the files File > Open… and drag and drop read for it in the background, the page embedding
    /// it (`?host=parent`), and the browser's `prefers-reduced-motion` (it stays live, so View ▸
    /// Accessibility ▸ Animation Effects ▸ Use System Setting follows a change).
    pub(crate) struct Shell(
        Rc<RefCell<CadApp>>,
        crate::upload::Inbox,
        Option<(crate::host::Bridge, crate::host::DirtyReport)>,
        Option<web_sys::MediaQueryList>,
        crate::fonts::Fonts,
        Option<crate::leave_guard::LeaveGuard>,
        Option<crate::fullscreen::WebFullscreen>,
        pub(crate) egui::Context,
        #[allow(dead_code)] Option<FlushListeners>,
    );

    impl eframe::App for Shell {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            let Ok(mut app) = self.0.try_borrow_mut() else { return };
            if self.6.as_ref().and_then(crate::fullscreen::WebFullscreen::take_event) == Some(false)
                && app.fullscreen.phase == cadcraft_ui_egui::fullscreen::Phase::Requested
            {
                cadcraft_ui_egui::fullscreen::rejected(&mut app);
            }
            self.1.take_dropped(ctx);
            self.1.drain(&mut app);
            self.4.frame(&mut app);
            app.system_animations = self.3.as_ref().map(|reduced| !reduced.matches());
            app.logic(ctx);
            if let Some(guard) = &self.5 {
                guard.set(app.has_unsaved_drawings());
            }
            if let Some((bridge, reported)) = &self.2 {
                bridge.report_dirty(&app, reported);
            }
        }
        fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            if let Ok(mut app) = self.0.try_borrow_mut() {
                app.raw_input_hook(raw);
            }
            cadcraft_ui_egui::cmdline::capture_tab(ctx, raw);
        }
        /// Preferences (interface theme and language) survive reloads; egui state is not kept.
        fn save(&mut self, storage: &mut dyn eframe::Storage) {
            let Ok(app) = self.0.try_borrow() else { return };
            storage.set_string(cadcraft_ui_egui::options::PROFILE_KEY, app.profile_json());
            storage.set_string(cadcraft_ui_egui::PREFS_KEY, app.prefs_json());
            storage.set_string(cadcraft_ui_egui::autosave::PREFS_KEY, cadcraft_ui_egui::autosave::prefs_json());
        }
        fn persist_egui_memory(&self) -> bool {
            false
        }
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let Ok(mut app) = self.0.try_borrow_mut() else { return };
            app.ui(ui);
            if let Some((bridge, reported)) = &self.2 {
                bridge.report_dirty(&app, reported);
            }
        }
    }

    struct FlushListeners {
        window: web_sys::Window,
        document: web_sys::Document,
        on_hide: Closure<dyn FnMut()>,
        on_visibility: Closure<dyn FnMut()>,
    }

    impl Drop for FlushListeners {
        fn drop(&mut self) {
            let _ = self.window.remove_event_listener_with_callback("pagehide", self.on_hide.as_ref().unchecked_ref());
            let _ = self.document.remove_event_listener_with_callback("visibilitychange", self.on_visibility.as_ref().unchecked_ref());
        }
    }

    fn flush_when_leaving(app: &Rc<RefCell<CadApp>>, ctx: &egui::Context) -> Option<FlushListeners> {
        let window = web_sys::window()?;
        let document = window.document()?;
        let flush = |app: &Rc<RefCell<CadApp>>, ctx: &egui::Context| {
            if let Ok(mut app) = app.try_borrow_mut() {
                app.browser_autosave_urgent(ctx);
                if app.ws.store.is_some()
                    && app.ws.needs_flush()
                    && let Err(error) = app.commit_workspace_now()
                {
                    log::warn!("workspace: {error}");
                }
            }
        };
        let on_hide = {
            let app = Rc::clone(app);
            let ctx = ctx.clone();
            Closure::<dyn FnMut()>::new(move || flush(&app, &ctx))
        };
        let on_visibility = {
            let app = Rc::clone(app);
            let ctx = ctx.clone();
            let document = document.clone();
            Closure::<dyn FnMut()>::new(move || {
                if document.hidden() {
                    flush(&app, &ctx);
                }
            })
        };
        let listeners = FlushListeners { window, document, on_hide, on_visibility };
        listeners.window.add_event_listener_with_callback("pagehide", listeners.on_hide.as_ref().unchecked_ref()).ok()?;
        listeners.document.add_event_listener_with_callback("visibilitychange", listeners.on_visibility.as_ref().unchecked_ref()).ok()?;
        Some(listeners)
    }

    fn query() -> String {
        web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
    }

    fn query_param(key: &str) -> Option<String> {
        web_sys::UrlSearchParams::new_with_str(&query()).ok()?.get(key)
    }

    /// No file picker on the web: Save / Save As from the menu (and the close prompt) download the
    /// drawing under its current name; `saveas {"path": "name.dxf"}` picks another name.
    fn services() -> Services {
        Services { pick_save: Some(Box::new(|name: &str| Some(name.to_string()))), ..Services::default() }
    }

    fn print_clock() -> u64 {
        let now = js_sys::Date::now();
        if now.is_finite() && now >= 0.0 { now as u64 } else { 0 }
    }

    pub fn start() {
        eframe::WebLogger::init(log::LevelFilter::Info).ok();
        cadcraft_engine::cmd::ubplot::set_clock(|| {
            let now = web_sys::js_sys::Date::now();
            if now.is_finite() && now >= 0.0 { now as u64 } else { 0 }
        });
        cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
            read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
            write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
            plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|e| e.to_string())),
        });
        cadcraft_engine::cmd::file::set_budget_plot(|d, space, opts, cancel, max_prims| {
            cadcraft_io::pdf::plot_with_budget(d, space, opts, Some(cancel), Some(max_prims)).map_err(|error| error.to_string())
        });
        cadcraft_engine::cmd::file::set_merge_pdf(|parts, labels, title| {
            cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| error.to_string())
        });
        cadcraft_engine::cmd::file::set_write_as(|d, name, o| {
            cadcraft_io::write_with(d, name, &cadcraft_io::SaveOptions { version: o.version, binary: o.binary }).map_err(|e| e.to_string())
        });
        // AutoLISP's entget, entmake and entmod see entities as DXF groups.
        cadcraft_engine::cmd::file::set_entity_codec(cadcraft_engine::cmd::file::EntityCodec {
            groups: cadcraft_io::entity_groups,
            entity: cadcraft_io::entity_from_groups,
        });
        // `?host=parent`: a same-origin page framing CADCraft (such as a Nextcloud app) opens
        // drawings in it and stores what it saves. Otherwise there is no file system here: saved
        // files reach the user as browser downloads.
        let bridge = if query_param("host").as_deref() == Some("parent") { crate::host::Bridge::new() } else { None };
        match &bridge {
            Some(bridge) => {
                bridge.install();
                cadcraft_engine::cmd::file::set_deliver(crate::host::deliver);
            }
            None => cadcraft_engine::cmd::file::set_deliver(crate::download::download),
        }
        let app_bridge = bridge.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Some(document) = web_sys::window().and_then(|w| w.document()) else { return };
            let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
                log::error!("missing <canvas id=\"{CANVAS_ID}\">");
                return;
            };
            let mut options = eframe::WebOptions::default();
            if query().contains("webgl")
                && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
            {
                create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
            }
            let profile_store = crate::idb_profile_store::open().await;
            let job_store = crate::job_store::BrowserKv::open().await;
            let result = eframe::WebRunner::new()
                .start(
                    canvas,
                    options,
                    Box::new(move |cc| {
                        // Embedded, the parent sends the drawing: start on the Start tab, not Drawing1.
                        let session = if app_bridge.is_some() { Session::empty() } else { Session::new() };
                        let mut app = CadApp::new(session, services());
                        // A framing page or the sample link wants a drawing, not a first-run question.
                        app.skip_first_run_picker = query_param("host").as_deref() == Some("parent") || query().contains("sample");
                        app.system_languages =
                            web_sys::window().map(|w| w.navigator().languages().iter().filter_map(|v| v.as_string()).collect()).unwrap_or_default();
                        let fonts = crate::fonts::Fonts::new(&cc.egui_ctx);
                        fonts.load(&mut app);
                        app.fontmgr.local_supported = crate::fonts::Fonts::can_list_local();
                        if let Some(profile) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::options::PROFILE_KEY)) {
                            app.load_profile(&profile);
                        }
                        if let Some(prefs) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::PREFS_KEY)) {
                            app.load_prefs(&prefs);
                        }
                        if let Some(prefs) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::autosave::PREFS_KEY)) {
                            cadcraft_ui_egui::autosave::load_prefs(&prefs);
                        }
                        // A workspace can override the language and theme loaded from preferences.
                        // Blocked storage still gets a store, one that reports every save as failed.
                        let safe_mode = query_param("safe").is_some();
                        match profile_store {
                            Ok(mut opened) => {
                                opened.profile.repaint_on_external_change(&cc.egui_ctx);
                                if safe_mode {
                                    app.attach_profile_store_safe(Box::new(opened.profile));
                                } else {
                                    app.attach_profile_store(Box::new(opened.profile));
                                }
                                opened.autosave.set_context(&cc.egui_ctx);
                                app.enable_browser_autosave(Box::new(opened.autosave), opened.instance, opened.leftovers);
                            }
                            Err(error) => {
                                log::warn!("workspace: IndexedDB unavailable: {}", error.message());
                                let fallback = if safe_mode { crate::profile_store::open_safe() } else { crate::profile_store::open() };
                                let store: Box<dyn cadcraft_ui_egui::wsprofile::store::ProfileStore> = match fallback {
                                    Some(mut store) => {
                                        store.repaint_on_external_change(&cc.egui_ctx);
                                        Box::new(store)
                                    }
                                    None => Box::new(cadcraft_ui_egui::wsprofile::store::MemoryStore { fail_writes: true, ..Default::default() }),
                                };
                                if safe_mode {
                                    app.attach_profile_store_safe(store);
                                } else {
                                    app.attach_profile_store(store);
                                }
                                let notice = cadcraft_ui_egui::tl!("Saving in limited browser storage");
                                let message = app.ws.message.get_or_insert_with(String::new);
                                if !message.is_empty() {
                                    message.push(' ');
                                }
                                message.push_str(&notice);
                                app.set_status(notice);
                            }
                        }
                        match job_store {
                            Ok(kv) => {
                                app.session.set_job_store(Box::new(crate::job_store::IdbJobStore::new(kv, crate::download::download)));
                                app.job_pump = cadcraft_ui_egui::ubplot::worker::JobPump::with_clock(&mut app.session, print_clock);
                            }
                            Err(error) => log::warn!("print jobs: IndexedDB unavailable: {error}"),
                        }
                        if query_param("lang").as_deref() == Some("qps") {
                            let _ = app.run("ui.language", serde_json::json!({"lang": "qps"}));
                        }
                        app.session.set_print_profile_store(Box::new(cadcraft_engine::sheetplan::profile::MemoryPrintProfileStore::default()));
                        app.session.echo("Print settings last for this visit");
                        app.set_status(cadcraft_ui_egui::tl!("Print settings last for this visit"));
                        // `?cpu`: no render state for the app, so the canvas draws with egui shapes.
                        if !crate::wants_cpu(&query())
                            && let Some(rs) = &cc.wgpu_render_state
                        {
                            // eframe's web runner has no stencil buffer: clipped layout
                            // viewports stay in the layout's display list.
                            app.set_wgpu(rs, None);
                        }
                        if query().contains("sample") {
                            let _ = app.run("ui.sample", serde_json::json!({}));
                        }
                        // No paths on the web: OPEN asks the browser's file picker, which reads the
                        // chosen file in the background (`upload.rs`), so the picker returns none.
                        let inbox = crate::upload::Inbox::new(&cc.egui_ctx);
                        let picker = inbox.clone();
                        app.services.pick_open = Some(Box::new(move || {
                            picker.pick();
                            None
                        }));
                        let picker = inbox.clone();
                        app.services.pick_open_kind = Some(Box::new(move |kind| {
                            let purpose = if kind.extensions == cadcraft_ui_egui::wsprofile::WORKSPACE_FILE.extensions {
                                crate::upload_route::Purpose::Workspace
                            } else {
                                crate::upload_route::Purpose::Drawing
                            };
                            picker.pick_for(purpose);
                            None
                        }));
                        let host = app_bridge.map(|bridge| {
                            bridge.listen(inbox.clone());
                            bridge.post_ready();
                            (bridge, crate::host::DirtyReport::default())
                        });
                        let reduced_motion = web_sys::window().and_then(|w| w.match_media("(prefers-reduced-motion: reduce)").ok().flatten());
                        let leave_guard =
                            if query_param("host").as_deref() == Some("parent") { None } else { crate::leave_guard::LeaveGuard::install() };
                        if let Some(guard) = &leave_guard {
                            guard.set(app.has_unsaved_drawings());
                        }
                        let app = Rc::new(RefCell::new(app));
                        let flush_listeners = flush_when_leaving(&app, &cc.egui_ctx);
                        let mut shell = Shell(app, inbox, host, reduced_motion, fonts, leave_guard, None, cc.egui_ctx.clone(), flush_listeners);
                        let fullscreen = match crate::fullscreen::install(&shell) {
                            Ok(fullscreen) => fullscreen,
                            Err(error) => {
                                log::warn!("fullscreen: {error:?}");
                                crate::fullscreen::WebFullscreen::default()
                            }
                        };
                        if let Ok(mut app) = shell.0.try_borrow_mut() {
                            app.services.fullscreen = Some(Box::new(fullscreen.clone()));
                        }
                        shell.6 = Some(fullscreen);
                        Ok(Box::new(shell))
                    }),
                )
                .await;
            if let (Err(e), Some(bridge)) = (&result, &bridge) {
                bridge.post_failed(&format!("{e:?}"));
            }
            if let Some(el) = document.get_element_by_id(LOADING_ID) {
                match result {
                    Ok(()) => el.remove(),
                    Err(e) => el.set_inner_html(&format!("<p>UbotCAD failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
                }
            }
        });
    }
}

/// True when the URL query (`?cpu&sample`) has a `cpu` flag: the CPU canvas path is requested.
#[cfg(any(target_arch = "wasm32", test))]
fn wants_cpu(query: &str) -> bool {
    query.trim_start_matches('?').split('&').any(|p| p == "cpu" || p.starts_with("cpu="))
}

#[cfg(any(target_arch = "wasm32", test))]
pub mod drawing_autosave;
#[cfg(any(target_arch = "wasm32", test))]
mod font_index;
#[cfg(target_arch = "wasm32")]
mod fonts;
#[cfg(target_arch = "wasm32")]
pub mod idb;
#[cfg(any(target_arch = "wasm32", test))]
pub mod idb_profile_store;
#[cfg(any(target_arch = "wasm32", test))]
mod idb_schema;
#[cfg(any(target_arch = "wasm32", test))]
pub mod job_store;
#[cfg(any(target_arch = "wasm32", test))]
mod profile_store;
#[cfg(target_arch = "wasm32")]
mod upload;
#[cfg(any(target_arch = "wasm32", test))]
mod upload_route;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("cadcraft-web only runs in the browser: build it with `trunk build --release` in apps/cadcraft-web");
}

#[cfg(test)]
mod tests {
    use super::wants_cpu;

    #[test]
    fn cpu_flag_is_detected() {
        assert!(wants_cpu("?cpu"));
        assert!(wants_cpu("?cpu&sample"));
        assert!(wants_cpu("?sample&cpu"));
        assert!(!wants_cpu(""));
        assert!(!wants_cpu("?sample"));
        assert!(!wants_cpu("?webgl&sample"));
        assert!(!wants_cpu("?gpucpu"));
    }
}

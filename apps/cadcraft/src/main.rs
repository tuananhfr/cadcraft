//! CADCraft desktop app.
//!
//! Usage: `cadcraft [--control <port>] [--sample] [files…]`
//!
//! `--control <port>` (or `CADCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server:
//! `{"id":1,"method":"cmdline.input","params":{"text":"circle 0,0 5"}}` → `{"id":1,"ok":true,…}`.
//! See `cadcraft_ui_egui::control` for the methods.
//!
//! `CADCRAFT_VSYNC=1` presents with vsync on Linux/BSD, where the default is the low-latency
//! present mode (`CADCRAFT_VSYNC=0` selects that elsewhere); see `cadcraft_ui_egui::gpu::surface_config`.
//!
//! `log` records go to standard error and `<settings dir>/logs/cadcraft.log` (`RUST_LOG` sets the
//! levels); see [`logging`] and [`config_dir`].
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod control_server;
mod desktop_theme;
#[cfg(any(target_os = "windows", test))]
mod graphics;
mod logging;
#[cfg(target_os = "macos")]
mod native_menu;
mod profile_store;
mod spacemouse;
mod system_animations;
mod workspace;

use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, FileKind, Services, i18n};

struct App {
    cad: CadApp,
    spacemouse: spacemouse::SpaceMouse,
    /// The desktop's light/dark choice where winit reports none (Linux, the BSDs).
    desktop_theme: desktop_theme::DesktopTheme,
    /// Whether the OS shows animations, for the System choice of View ▸ Accessibility ▸ Animation Effects.
    system_animations: system_animations::SystemAnimations,
    /// The fonts the user added, kept in `<settings dir>/fonts`.
    fonts: Option<cadcraft_ui_egui::fontstore::FontStore>,
    #[cfg(target_os = "macos")]
    native_menu: Option<native_menu::NativeMenu>,
}

impl App {
    /// Keep the fonts the user added or removed in the settings folder.
    fn persist_fonts(&mut self) {
        if let Some(store) = &self.fonts {
            store.sync(&mut self.cad);
        }
    }

    /// The CadApp tick saves on its own; this keeps a change made just before closing (inside the
    /// autosave delay, or after a failed save).
    fn flush_workspace(&mut self) {
        if self.cad.ws.store.is_some()
            && self.cad.ws.needs_flush()
            && let Err(error) = self.cad.commit_workspace_now()
        {
            eprintln!("CADCraft workspace: {error}");
        }
    }
}

/// View ▸ Theme ▸ System resolves against `raw.system_theme`, which winit leaves empty on Linux
/// and the BSDs: the desktop's answer, where it gives one, stands in for it.
fn follow_desktop(raw: &mut egui::RawInput, desktop: Option<egui::Theme>) {
    if desktop.is_some() {
        raw.system_theme = desktop;
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            if self.native_menu.is_none() && std::env::var_os("CADCRAFT_NO_NATIVE_MENU").is_none() {
                self.native_menu = Some(native_menu::NativeMenu::install(&mut self.cad));
            }
            if let Some(m) = &mut self.native_menu {
                m.poll(&mut self.cad, ctx);
            }
        }
        self.cad.system_animations = self.system_animations.current();
        self.cad.logic(ctx);
        self.spacemouse.poll(&mut self.cad, ctx);
        if std::mem::take(&mut self.cad.quit_requested) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        follow_desktop(raw, self.desktop_theme.current());
        self.cad.raw_input_hook(raw);
        cadcraft_ui_egui::cmdline::capture_tab(ctx, raw);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.cad.ui(ui);
        self.persist_fonts();
    }
    /// A normal exit: the workspace is saved, and this run's autosave files are not needed (a
    /// crash leaves them for recovery).
    fn on_exit(&mut self) {
        self.flush_workspace();
        self.cad.shutdown_autosave();
    }
    /// Preferences (the interface theme) survive restarts; window and egui state are not kept.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(cadcraft_ui_egui::options::PROFILE_KEY, self.cad.profile_json());
        storage.set_string(cadcraft_ui_egui::PREFS_KEY, self.cad.prefs_json());
        storage.set_string(cadcraft_ui_egui::autosave::PREFS_KEY, cadcraft_ui_egui::autosave::prefs_json());
    }
    fn persist_egui_memory(&self) -> bool {
        false
    }
}

/// File format hooks for the engine (it stays I/O-agnostic).
pub fn install_io() {
    #[cfg(not(target_arch = "wasm32"))]
    cadcraft_engine::sheetplan::registry_store::set_registry_store_factory(|| {
        Ok(Box::new(cadcraft_engine::sheetplan::registry_store::DirRegistryStore::settings_default()?))
    });
    #[cfg(not(target_arch = "wasm32"))]
    cadcraft_engine::cmd::ubplot::set_clock(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
            .unwrap_or(0)
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
}

/// Add Font...: a font file chosen in the system dialog, with its contents.
fn pick_font() -> cadcraft_ui_egui::fontmgr::PickFontResult {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Fonts (TTF, OTF, TTC, SHX)", &["ttf", "otf", "ttc", "otc", "shx"])
        .add_filter(i18n::t("All files"), &["*"])
        .pick_file()
    else {
        return Ok(None);
    };
    let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else { return Ok(None) };
    let bytes = cadcraft_ui_egui::fontstore::read_font_file(&path).map_err(|e| e.to_string())?;
    Ok(Some((name, bytes)))
}

fn services() -> Services {
    Services {
        fullscreen: None,
        pick_open: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter(i18n::t("Drawings (DWG, DXF)"), &["dwg", "dxf", "dwt"])
                .add_filter(i18n::t("SVG drawings (SVG, SVGZ)"), &["svg", "svgz"])
                .add_filter(i18n::t("All files"), &["*"])
                .pick_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_save: Some(Box::new(|name: &str| {
            filters_for_save(name)
                .iter()
                .fold(rfd::FileDialog::new().set_file_name(name), |d, (label, ext)| d.add_filter(i18n::t(label), &[*ext]))
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_open_kind: Some(Box::new(|kind| {
            filters_for_kind(kind, true)
                .iter()
                .fold(rfd::FileDialog::new(), |d, (label, exts)| d.add_filter(i18n::t(label), exts))
                .pick_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
        pick_save_kind: Some(Box::new(|name, kind| {
            filters_for_kind(kind, false)
                .iter()
                .fold(rfd::FileDialog::new().set_file_name(name), |d, (label, exts)| d.add_filter(i18n::t(label), exts))
                .save_file()
                .map(|p| p.to_string_lossy().to_string())
        })),
    }
}

/// Filters for a non-drawing dialog. Save lists only the kind, so macOS keeps its extension.
fn filters_for_kind(kind: &FileKind, open: bool) -> Vec<(&'static str, &'static [&'static str])> {
    let mut filters = vec![(kind.label, kind.extensions)];
    if open {
        filters.push(("All files", &["*"]));
    }
    filters
}

/// File types for the Save dialog, the suggested name's type first: Windows adds the first
/// filter's extension, and macOS accepts only listed extensions, replacing any other with the
/// first one. DWG must be listed for a `.dwg` name to survive, and a drawing opened from DWG
/// then defaults to DWG; other names keep DXF first.
fn filters_for_save(name: &str) -> [(&'static str, &'static str); 6] {
    let mut filters =
        [("Drawing (DXF)", "dxf"), ("Drawing (DWG)", "dwg"), ("Drawing Template (DWT)", "dwt"), ("SVG", "svg"), ("PNG", "png"), ("PDF", "pdf")];
    let lower = name.to_ascii_lowercase();
    filters.sort_by_key(|(_, ext)| !lower.ends_with(&format!(".{ext}")));
    filters
}

/// The per-user settings directory (it holds `logs/` and `autosave/`): `CADCRAFT_CONFIG_DIR` if set, else
/// `$XDG_CONFIG_HOME/ubotcad` or `~/.config/ubotcad` on Linux and the BSDs,
/// `~/Library/Application Support/UbotCAD` on macOS, `%APPDATA%\UbotCAD` on Windows.
fn config_dir() -> Option<std::path::PathBuf> {
    config_dir_from(|k| std::env::var_os(k))
}

/// [`config_dir`] with the environment passed in (`var` looks a variable up). An empty variable
/// counts as unset, so a blank value never means the working directory.
fn config_dir_from(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<std::path::PathBuf> {
    let set = |k: &str| var(k).filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    if let Some(dir) = set("CADCRAFT_CONFIG_DIR") {
        return Some(dir);
    }
    if cfg!(target_os = "macos") {
        set("HOME").map(|h| h.join("Library/Application Support/UbotCAD"))
    } else if cfg!(windows) {
        set("APPDATA").map(|a| a.join("UbotCAD"))
    } else {
        // The XDG spec ignores a relative XDG_CONFIG_HOME.
        set("XDG_CONFIG_HOME").filter(|p| p.is_absolute()).or_else(|| set("HOME").map(|h| h.join(".config"))).map(|c| c.join("ubotcad"))
    }
}

const APP_ID: &str = "ai.storyteller.cadcraft";

/// Stencil bits of the window's render pass (see `NativeOptions::stencil_buffer`).
const STENCIL_BITS: u8 = 8;

#[cfg(target_os = "macos")]
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/cadcraft-macos-512.png");
#[cfg(not(target_os = "macos"))]
const APP_ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/cadcraft-256.png");

fn app_icon() -> Option<egui::IconData> {
    eframe::icon_data::from_png_bytes(APP_ICON_PNG).map_err(|e| log::warn!("app icon: {e}")).ok()
}

fn version_string() -> String {
    let sha = option_env!("CADCRAFT_BUILD_SHA").unwrap_or("dev");
    format!("cadcraft {} ({sha})", env!("CARGO_PKG_VERSION"))
}

/// The control port from `--control <port>` or `CADCRAFT_CONTROL_PORT` (`source`); a value that is
/// not a port is logged and ignored instead of silently starting no control channel.
fn control_port_from(source: &str, value: Option<String>) -> Option<u16> {
    let value = value?;
    let port = value.trim().parse().ok();
    if port.is_none() {
        log::warn!("{source}: {value:?} is not a port number; the control channel is off");
    }
    port
}

/// `--control` followed by `value`: a bare `--control` (last argument) keeps the port already
/// read from `CADCRAFT_CONTROL_PORT` (`current`) instead of turning the control channel off.
fn control_flag(current: Option<u16>, value: Option<String>) -> Option<u16> {
    if value.is_some() {
        return control_port_from("--control", value);
    }
    match current {
        Some(port) => log::warn!("--control: no port given; using CADCRAFT_CONTROL_PORT ({port})"),
        None => log::warn!("--control: no port given; the control channel is off"),
    }
    current
}

#[derive(Default)]
struct LaunchArgs {
    control_port: Option<u16>,
    files: Vec<String>,
    sample: bool,
    safe_mode: bool,
    version: bool,
}

fn parse_launch_args(mut args: impl Iterator<Item = String>, control_port: Option<u16>) -> LaunchArgs {
    let mut launch = LaunchArgs { control_port, ..LaunchArgs::default() };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--control" => launch.control_port = control_flag(launch.control_port, args.next()),
            "--sample" => launch.sample = true,
            "--safe-mode" => launch.safe_mode = true,
            "--version" | "-V" => {
                launch.version = true;
                break;
            }
            _ => launch.files.push(arg),
        }
    }
    launch
}

fn main() -> eframe::Result {
    // First, so every start-up record is captured; see `logging`.
    let logger = logging::install();
    // lossy, so a value that is not Unicode is reported like any other bad port instead of ignored
    let control_port =
        control_port_from("CADCRAFT_CONTROL_PORT", std::env::var_os("CADCRAFT_CONTROL_PORT").map(|p| p.to_string_lossy().into_owned()));
    let LaunchArgs { control_port, files, sample, safe_mode, version } = parse_launch_args(std::env::args().skip(1), control_port);
    if version {
        println!("{}", version_string());
        return Ok(());
    }
    // The log file lives under the settings directory; opened after the arguments, so `--version`
    // leaves no file behind. Records logged until now are written to it first.
    if let Some(logger) = logger {
        match config_dir().map(|d| logger.attach_dir(&d.join(logging::LOG_DIR))) {
            Some(Ok(path)) => log::info!("{} ({} {}), log file {}", version_string(), std::env::consts::OS, std::env::consts::ARCH, path.display()),
            Some(Err(e)) => log::warn!("no log file: {e}"),
            None => {
                logger.stderr_only();
                log::warn!("no log file: no settings directory (set CADCRAFT_CONFIG_DIR)");
            }
        }
    }
    install_io();
    let surface = cadcraft_ui_egui::gpu::surface_config(std::env::var("CADCRAFT_VSYNC").ok().as_deref(), cfg!(all(unix, not(target_os = "macos"))));
    let mut options = eframe::NativeOptions {
        // The crosshair is drawn by the app: present without a queue on Linux/BSD, where X11
        // swapchains hold two frames back under vsync (`CADCRAFT_VSYNC=1` restores vsync).
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(surface),
        viewport: egui::ViewportBuilder::default()
            .with_title("UbotCAD")
            .with_inner_size([1600.0, 1000.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false)
            .with_app_id(APP_ID),
        persist_window: false,
        // A stencil buffer clips the contents of clipped layout viewports on the GPU.
        stencil_buffer: STENCIL_BITS,
        ..Default::default()
    };
    if let Some(icon) = app_icon() {
        options.viewport = options.viewport.with_icon(icon);
    }
    // Before eframe creates the wgpu instance: default Windows to DirectX 12 with FXC (see graphics.rs).
    #[cfg(target_os = "windows")]
    graphics::configure(&mut options, eframe::wgpu::Backends::from_env(), eframe::wgpu::Dx12Compiler::from_env());
    // Command previews (COPY, MOVE...) of large selections are tessellated by egui, whose index
    // buffer outgrew wgpu's default 256 MiB cap; the validation error is fatal. Ask for what the
    // adapter actually supports.
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        let base = std::sync::Arc::clone(&create.device_descriptor);
        create.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut d = base(adapter);
            d.required_limits.max_buffer_size = d.required_limits.max_buffer_size.max(adapter.limits().max_buffer_size);
            d
        });
    }
    // winit has no file drag-and-drop on Wayland (only on X11), so dropping files from the file
    // manager showed a "no" cursor. Run through XWayland when it's there; CADCRAFT_WAYLAND=1
    // keeps the native Wayland backend.
    #[cfg(all(unix, not(target_os = "macos")))]
    if std::env::var_os("DISPLAY").is_some() && std::env::var_os("CADCRAFT_WAYLAND").is_none() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    eframe::run_native(
        "UbotCAD",
        options,
        Box::new(move |cc| {
            let mut app = CadApp::new(Session::empty(), services());
            app.system_languages = std::env::var("CADCRAFT_LOCALE").map(|s| vec![s]).unwrap_or_else(|_| sys_locale::get_locales().collect());
            if let Some(store) = cadcraft_engine::sheetplan::queue::DirJobStore::settings_default() {
                app.job_pump = cadcraft_ui_egui::ubplot::worker::JobPump::new(store.root().to_path_buf());
                app.session.set_job_store(Box::new(store));
            }
            // The user profile (Options): drafting settings, selection sizes, drawing-area colours.
            if let Some(profile) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::options::PROFILE_KEY)) {
                app.load_profile(&profile);
            }
            if let Some(prefs) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::PREFS_KEY)) {
                app.load_prefs(&prefs);
            }
            if let Some(prefs) = cc.storage.and_then(|s| s.get_string(cadcraft_ui_egui::autosave::PREFS_KEY)) {
                cadcraft_ui_egui::autosave::load_prefs(&prefs);
            }
            // A scripted or sample run wants a drawing, not a first-run question (D4).
            app.skip_first_run_picker = sample || control_port.is_some();
            // After the preferences: a profile that names a theme or language overrides them.
            if safe_mode {
                let store: Box<dyn cadcraft_ui_egui::wsprofile::store::ProfileStore> = match config_dir() {
                    Some(dir) => Box::new(profile_store::FileProfileStore::new(dir.join("workspace"))),
                    None => Box::new(cadcraft_ui_egui::wsprofile::store::MemoryStore::default()),
                };
                app.attach_profile_store_safe(store);
            } else if let Some(opened) = workspace::open_default() {
                app.attach_profile_store(Box::new(opened.store));
                if let Some(notice) = opened.notice {
                    eprintln!("CADCraft workspace: {notice}");
                    app.set_status(notice);
                }
            }
            if std::env::var("CADCRAFT_LOCALE").as_deref() == Ok("qps") {
                let _ = app.run("ui.language", serde_json::json!({"lang": "qps"}));
            }
            // Autosave files (SAVEFILEPATH) default to `<settings dir>/autosave`.
            if let Some(dir) = config_dir() {
                app.enable_autosave(&dir.join("autosave"));
                app.session
                    .set_print_profile_store(Box::new(cadcraft_engine::sheetplan::profile::DirPrintProfileStore::new(dir.join("print-profiles"))));
                // Startup LISP (acad.lsp, acaddoc.lsp) is read from `<settings>/support`; the
                // folder is made so it is there to put them in.
                let support = dir.join("support");
                if let Err(e) = std::fs::create_dir_all(&support) {
                    log::warn!("no LISP support folder {}: {e}", support.display());
                }
                let lisp_out = dir.join("lisp-out");
                if let Err(e) = std::fs::create_dir_all(&lisp_out) {
                    log::warn!("no LISP output folder {}: {e}", lisp_out.display());
                }
                cadcraft_engine::lisp::sandbox::set_write_roots(vec![support.clone(), lisp_out]);
                app.session.set_lisp_support_dir(Some(support));
            }
            app.fontmgr.pick_file = Some(Box::new(pick_font));
            let fonts = config_dir().map(|d| cadcraft_ui_egui::fontstore::FontStore::new(d.join("fonts")));
            if let Some(store) = &fonts {
                store.load(&mut app);
            }
            app.integrated_titlebar = cfg!(target_os = "macos");
            if surface.present_mode == eframe::wgpu::PresentMode::AutoNoVsync {
                app.frame_cap = Some(cadcraft_ui_egui::gpu::FrameCap::new(240));
            }
            if let Some(rs) = &cc.wgpu_render_state {
                app.set_wgpu(rs, eframe::egui_wgpu::depth_format_from_bits(0, STENCIL_BITS));
            }
            if let Some(port) = control_port {
                let rx = control_server::start(port, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            if sample {
                let _ = app.run("ui.sample", serde_json::json!({}));
            }
            for f in &files {
                app.open_path(f);
            }
            if app.session.docs.is_empty() {
                app.session.new_drawing(false);
            }
            let ctx = cc.egui_ctx.clone();
            let desktop_theme = desktop_theme::DesktopTheme::start(move || ctx.request_repaint());
            let ctx = cc.egui_ctx.clone();
            let system_animations = system_animations::SystemAnimations::start(move || ctx.request_repaint());
            Ok(Box::new(App {
                desktop_theme,
                system_animations,
                cad: app,
                spacemouse: spacemouse::SpaceMouse::start(cc.egui_ctx.clone()),
                fonts,
                #[cfg(target_os = "macos")]
                native_menu: None,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{config_dir_from, control_flag, control_port_from, filters_for_kind, filters_for_save, follow_desktop, parse_launch_args};
    use cadcraft_ui_egui::wsprofile::WORKSPACE_FILE;
    use std::ffi::OsString;
    use std::path::PathBuf;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| OsString::from(v))
    }

    #[test]
    fn save_dialog_offers_dwg_with_the_drawing_format_first() {
        let first = |name: &str| filters_for_save(name)[0].1;
        assert_eq!(first("Waco Winnelson Floorplan.dwg"), "dwg");
        assert_eq!(first("FLOORPLAN.DWG"), "dwg");
        // New drawings and DXF files keep DXF first.
        assert_eq!(first("Drawing1.dxf"), "dxf");
        assert_eq!(first("Drawing1"), "dxf");
        assert_eq!(first("plan.pdf"), "pdf");
        assert_eq!(first("office.dwt"), "dwt");
        for name in ["a.dwg", "b.dxf", "c"] {
            let exts: Vec<_> = filters_for_save(name).iter().map(|(_, e)| *e).collect();
            assert!(exts.contains(&"dwg") && exts.contains(&"dxf"), "{name}");
        }
    }

    #[test]
    fn workspace_dialogs_offer_json_first() {
        assert_eq!(filters_for_kind(&WORKSPACE_FILE, false), vec![("Workspace (JSON)", &["json"][..])]);
        assert_eq!(filters_for_kind(&WORKSPACE_FILE, true), vec![("Workspace (JSON)", &["json"][..]), ("All files", &["*"][..])]);
    }

    #[test]
    fn safe_mode_flag_is_parsed() {
        let args = ["--sample", "drawing.dxf", "--safe-mode", "--control", "7979"].into_iter().map(str::to_string);
        let launch = parse_launch_args(args, None);
        assert!(launch.safe_mode);
        assert!(launch.sample);
        assert_eq!(launch.files, ["drawing.dxf"]);
        assert_eq!(launch.control_port, Some(7979));
        assert!(!launch.version);
        assert!(!parse_launch_args(std::iter::empty(), None).safe_mode);
    }

    #[test]
    fn control_port_values_that_are_not_ports_are_ignored() {
        assert_eq!(control_port_from("--control", Some("7979".into())), Some(7979));
        assert_eq!(control_port_from("--control", Some(" 7979 ".into())), Some(7979));
        assert_eq!(control_port_from("--control", None), None);
        for bad in ["", "abc", "-1", "65536", "99999999999999999999", "\u{0}"] {
            assert_eq!(control_port_from("--control", Some(bad.into())), None, "{bad:?}");
        }
    }

    #[test]
    fn control_without_a_port_keeps_the_environment_port() {
        // `CADCRAFT_CONTROL_PORT=47321 cadcraft --control` (#844)
        assert_eq!(control_flag(Some(47321), None), Some(47321));
        assert_eq!(control_flag(None, None), None);
        // A port given on the command line still wins over the variable.
        assert_eq!(control_flag(Some(47321), Some("7979".into())), Some(7979));
        assert_eq!(control_flag(Some(47321), Some("abc".into())), None);
    }

    #[test]
    fn the_override_variable_wins_and_empty_values_are_unset() {
        let vars = [("CADCRAFT_CONFIG_DIR", "/tmp/cc"), ("HOME", "/home/u"), ("XDG_CONFIG_HOME", "/x"), ("APPDATA", "C:\\a")];
        assert_eq!(config_dir_from(env(&vars)), Some(PathBuf::from("/tmp/cc")));
        let empty = [("CADCRAFT_CONFIG_DIR", ""), ("HOME", ""), ("XDG_CONFIG_HOME", ""), ("APPDATA", "")];
        assert_eq!(config_dir_from(env(&empty)), None, "no usable variable means no directory, not the working directory");
        assert_eq!(config_dir_from(env(&[])), None);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_and_bsd_follow_xdg_config_home() {
        assert_eq!(config_dir_from(env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/home/u")])), Some(PathBuf::from("/x/ubotcad")));
        assert_eq!(config_dir_from(env(&[("HOME", "/home/u")])), Some(PathBuf::from("/home/u/.config/ubotcad")));
        // The XDG spec says a relative XDG_CONFIG_HOME is ignored.
        assert_eq!(config_dir_from(env(&[("XDG_CONFIG_HOME", "rel"), ("HOME", "/home/u")])), Some(PathBuf::from("/home/u/.config/ubotcad")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_application_support() {
        assert_eq!(config_dir_from(env(&[("HOME", "/Users/u")])), Some(PathBuf::from("/Users/u/Library/Application Support/UbotCAD")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_appdata() {
        assert_eq!(
            config_dir_from(env(&[("APPDATA", "C:\\Users\\u\\AppData\\Roaming")])),
            Some(PathBuf::from("C:\\Users\\u\\AppData\\Roaming").join("UbotCAD"))
        );
    }

    /// View ▸ Theme ▸ System on Linux: winit reports no theme, so the desktop's answer stands in;
    /// without one (or where winit answers and the desktop doesn't) the input is left alone.
    #[test]
    fn the_desktop_answers_for_the_system_theme() {
        let mut raw = egui::RawInput::default();
        follow_desktop(&mut raw, Some(egui::Theme::Light));
        assert_eq!(raw.system_theme, Some(egui::Theme::Light));
        let mut raw = egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() };
        follow_desktop(&mut raw, None);
        assert_eq!(raw.system_theme, Some(egui::Theme::Dark));
        let mut raw = egui::RawInput::default();
        follow_desktop(&mut raw, None);
        assert_eq!(raw.system_theme, None);
    }
}

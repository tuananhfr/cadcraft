//! Which controls the External References palette shows for its host: where saved paths can't be
//! followed on disk (the web), the ones that read files give way to Choose file…
//! ([`crate::xref_pick`]), and the actions on the drawing in memory stay.

use cadcraft_engine::cmd::registry::{CommandStatus, Target, option_status_on};

use crate::CadApp;

/// What the palette's host can do with xref files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Host {
    /// The host has a picker for an xref's file.
    pub can_pick: bool,
    /// Saved paths are followed on disk (Reload, Path, Attach and Open work).
    pub disk: bool,
}

impl Host {
    pub(crate) fn of(app: &CadApp) -> Self {
        let disk = option_status_on("-xref", "reload", Target::current()) == CommandStatus::Available;
        Host { can_pick: app.services.pick_xref_file.is_some(), disk }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Control {
    AttachDwg,
    ReloadAll,
    ChooseFile,
    /// Reload from the saved path, or (without a disk) from a picked file.
    Reload,
    Unload,
    Detach,
    Bind,
    Insert,
    Open,
}

/// The buttons above the list.
pub(crate) fn toolbar(host: Host) -> Vec<Control> {
    if host.disk { vec![Control::AttachDwg, Control::ReloadAll] } else { Vec::new() }
}

/// The selected xref's buttons, in order, for an xref with palette status `status`.
pub(crate) fn row(host: Host, status: &str) -> Vec<Control> {
    let loaded = matches!(status, "Loaded" | "Unreferenced");
    let mut out = Vec::new();
    if crate::xref_pick::offers_file(host.can_pick, status) {
        out.push(Control::ChooseFile);
    }
    if host.disk || (host.can_pick && !loaded) {
        out.push(Control::Reload);
    }
    out.extend([Control::Unload, Control::Detach, Control::Bind, Control::Insert]);
    if host.disk {
        out.push(Control::Open);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Control::*;

    const DESKTOP: Host = Host { can_pick: false, disk: true };
    const WEB: Host = Host { can_pick: true, disk: false };

    #[test]
    fn the_desktop_palette_is_unchanged() {
        assert_eq!(toolbar(DESKTOP), [AttachDwg, ReloadAll]);
        for status in ["Loaded", "Unreferenced", "Unloaded", "Not Found", "Unresolved", "Not loaded"] {
            assert_eq!(row(DESKTOP, status), [Reload, Unload, Detach, Bind, Insert, Open], "{status}");
        }
    }

    #[test]
    fn the_web_palette_picks_files_instead_of_reading_saved_paths() {
        assert!(toolbar(WEB).is_empty());
        for status in ["Not Found", "Unresolved"] {
            assert_eq!(row(WEB, status), [ChooseFile, Reload, Unload, Detach, Bind, Insert], "{status}");
        }
        // Unloaded comes back only from a picked file there.
        assert_eq!(row(WEB, "Unloaded"), [Reload, Unload, Detach, Bind, Insert]);
        for status in ["Loaded", "Unreferenced"] {
            assert_eq!(row(WEB, status), [Unload, Detach, Bind, Insert], "{status}");
        }
    }

    #[test]
    fn without_a_picker_or_a_disk_only_the_in_memory_actions_remain() {
        let none = Host { can_pick: false, disk: false };
        assert!(toolbar(none).is_empty());
        assert_eq!(row(none, "Not Found"), [Unload, Detach, Bind, Insert]);
    }

    #[test]
    fn a_desktop_host_with_a_picker_adds_choose_file_for_missing_xrefs() {
        let both = Host { can_pick: true, disk: true };
        assert_eq!(row(both, "Not Found"), [ChooseFile, Reload, Unload, Detach, Bind, Insert, Open]);
        assert_eq!(row(both, "Loaded"), [Reload, Unload, Detach, Bind, Insert, Open]);
    }
}

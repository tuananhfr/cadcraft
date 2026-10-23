use std::sync::OnceLock;

use cadcraft_doc::{DevicePaper, Orientation, PlotOutput, PlotterConfig};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrinterInfo {
    pub name: String,
    pub is_default: bool,
    pub offline: bool,
    pub papers: Vec<(String, f64, f64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrintOutcome {
    Spooled { printer: String },
    NotSupported(String),
}

#[derive(Clone, Copy)]
pub struct PrinterHost {
    pub list: fn() -> Result<Vec<PrinterInfo>, String>,
    pub print_pdf: fn(printer: &str, path: &str) -> PrintOutcome,
}

static HOST: OnceLock<PrinterHost> = OnceLock::new();
static REFRESH: OnceLock<fn() -> Result<Vec<PrinterInfo>, String>> = OnceLock::new();

pub fn set_printer_host(host: PrinterHost) {
    let _ = HOST.set(host);
}

pub fn set_printer_refresh_hook(refresh: fn() -> Result<Vec<PrinterInfo>, String>) {
    let _ = REFRESH.set(refresh);
}

pub fn printer_host() -> Option<PrinterHost> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        HOST.get().copied()
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

pub fn list_printers(refresh: bool) -> Result<Vec<PrinterInfo>, String> {
    let Some(host) = printer_host() else { return Ok(Vec::new()) };
    let list = if refresh { REFRESH.get().copied().unwrap_or(host.list) } else { host.list };
    list().map(|printers| printers.into_iter().take(256).collect())
}

pub(crate) fn find_printer(name: &str) -> Option<PrinterInfo> {
    list_printers(false).ok()?.into_iter().find(|printer| printer.name.eq_ignore_ascii_case(name.trim()))
}

pub(crate) fn plotter_config(printer: &PrinterInfo) -> PlotterConfig {
    let papers: Vec<_> = printer
        .papers
        .iter()
        .take(512)
        .filter(|(_, width, height)| width.is_finite() && height.is_finite() && (1.0..=15000.0).contains(width) && (1.0..=15000.0).contains(height))
        .map(|(name, width, height)| DevicePaper { name: name.clone(), width: *width, height: *height, margins: [0.0; 4] })
        .collect();
    PlotterConfig {
        name: printer.name.clone(),
        description: String::new(),
        output: PlotOutput::Pdf,
        resolution: 300,
        orientation: Orientation::Portrait,
        default_paper: papers.first().map(|paper| paper.name.clone()).unwrap_or_default(),
        papers,
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(clippy::unwrap_used)]
mod tests {
    #[test]
    fn printers_without_desktop_host_return_empty() {
        let mut session = crate::Session::empty();
        assert_eq!(session.execute("ubplot.printers", &serde_json::json!({})).unwrap(), serde_json::json!([]));
        assert!(session.log.iter().any(|line| line == "printers are only in the desktop app"));
    }
}

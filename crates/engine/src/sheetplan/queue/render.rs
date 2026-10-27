//! Primitive and output limits are memory proxies because Rust has no per-thread memory cap.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cadcraft_doc::{Drawing, Space};
use serde_json::{Value, json};

use super::PrintJob;
use crate::cmd::file::{budget_plot, merge_pdf};
use crate::cmd::ubplot::publish::sheet_pages_for_drawing;
use crate::sheetplan::Sheet;

#[derive(Clone, Debug)]
pub struct SheetBudget {
    pub cancel: Arc<AtomicBool>,
    pub max_prims: usize,
    pub max_bytes: usize,
    pub time_ms: u64,
}

impl SheetBudget {
    pub fn desktop() -> Self {
        Self { cancel: Arc::new(AtomicBool::new(false)), max_prims: 50_000_000, max_bytes: 512 * 1024 * 1024, time_ms: 300_000 }
    }

    pub fn web() -> Self {
        Self { cancel: Arc::new(AtomicBool::new(false)), max_prims: 10_000_000, max_bytes: 128 * 1024 * 1024, time_ms: 300_000 }
    }

    /// Callers use this wrapper so custom renderers receive the same deadline and output checks.
    pub fn render(&self, renderer: &dyn SheetRenderer, d: &Drawing, sheet: &Sheet) -> Result<Vec<u8>, RenderFailure> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(RenderFailure::Cancelled);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let watchdog = Watchdog::arm(self.cancel.clone(), std::time::Duration::from_millis(self.time_ms))?;
        let result = renderer.render(d, sheet, self);
        #[cfg(not(target_arch = "wasm32"))]
        if watchdog.disarm() {
            return Err(RenderFailure::TimedOut);
        }
        self.check_result(result)
    }

    fn check_result(&self, result: Result<Vec<u8>, RenderFailure>) -> Result<Vec<u8>, RenderFailure> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(RenderFailure::Cancelled);
        }
        let bytes = result?;
        if bytes.len() > self.max_bytes {
            return Err(RenderFailure::TooLarge(format!("SHEET_TOO_LARGE: {} output bytes exceed {}", bytes.len(), self.max_bytes)));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderFailure {
    Cancelled,
    TimedOut,
    TooLarge(String),
    Error(String),
}

pub trait SheetRenderer {
    fn render(&self, d: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure>;
    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, RenderFailure>;

    /// A renderer carrying `job`'s plot style, used instead of `self`; `None` renders with `self` as is.
    fn job_renderer(&self, _job: &PrintJob) -> Option<PdfSheetRenderer> {
        None
    }
}

/// Renders sheets to PDF with a job's plot style table and lineweight setting; `None` keeps each page's own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PdfSheetRenderer {
    /// A table name, or "None" for colours as displayed.
    pub plot_style_table: Option<String>,
    pub lineweights: Option<bool>,
}

impl PdfSheetRenderer {
    pub fn for_job(job: &PrintJob) -> Self {
        Self { plot_style_table: job.plot_style_table.clone(), lineweights: job.lineweights }
    }
}

/// Adds the plot style options to a PDF plot's options; an explicit "None" overrides a layout's own table.
pub(crate) fn plot_style_options(options: &mut Value, table: Option<&str>, lineweights: Option<bool>) {
    let Some(object) = options.as_object_mut() else { return };
    if let Some(table) = table {
        object.insert("plotStyleTable".into(), json!(table));
    }
    if let Some(lineweights) = lineweights {
        object.insert("lineweights".into(), json!(lineweights));
    }
}

impl SheetRenderer for PdfSheetRenderer {
    fn render(&self, d: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(RenderFailure::Cancelled);
        }
        let pages = sheet_pages_for_drawing(d, sheet).map_err(RenderFailure::Error)?;
        let plot = budget_plot().ok_or_else(|| RenderFailure::Error("budgeted PDF plotting is not available in this build".into()))?;
        let mut options = json!({"pages": pages, "output": "pdf"});
        plot_style_options(&mut options, self.plot_style_table.as_deref(), self.lineweights);
        let drawing = crate::sheetplan::exclude::plot_drawing(d, std::slice::from_ref(sheet));
        let result = plot(&drawing, &Space::Model, &options, budget.cancel.clone(), budget.max_prims)
            .map_err(|error| if error.contains("SHEET_TOO_LARGE") { RenderFailure::TooLarge(error) } else { RenderFailure::Error(error) });
        budget.check_result(result)
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, RenderFailure> {
        let merge = merge_pdf().ok_or_else(|| RenderFailure::Error("PDF merging is not available in this build".into()))?;
        merge(parts, labels, title).map_err(RenderFailure::Error)
    }

    fn job_renderer(&self, job: &PrintJob) -> Option<PdfSheetRenderer> {
        Some(Self::for_job(job))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct Watchdog;

#[cfg(not(target_arch = "wasm32"))]
pub struct WatchdogGuard {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
    timed_out: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Watchdog {
    pub fn arm(cancel: Arc<AtomicBool>, deadline: std::time::Duration) -> Result<WatchdogGuard, RenderFailure> {
        let (stop, receiver) = std::sync::mpsc::channel();
        let timed_out = Arc::new(AtomicBool::new(false));
        let fired = timed_out.clone();
        let thread = std::thread::Builder::new()
            .name("sheet-watchdog".into())
            .spawn(move || {
                if matches!(receiver.recv_timeout(deadline), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) && !cancel.load(Ordering::Relaxed) {
                    fired.store(true, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                }
            })
            .map_err(|error| RenderFailure::Error(error.to_string()))?;
        Ok(WatchdogGuard { stop: Some(stop), thread: Some(thread), timed_out })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl WatchdogGuard {
    pub fn disarm(mut self) -> bool {
        self.stop();
        self.timed_out.load(Ordering::Relaxed)
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for WatchdogGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

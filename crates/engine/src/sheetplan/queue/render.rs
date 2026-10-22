//! Primitive and output limits are memory proxies because Rust has no per-thread memory cap.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cadcraft_doc::{Drawing, Space};
use serde_json::json;

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
}

pub struct PdfSheetRenderer;

impl SheetRenderer for PdfSheetRenderer {
    fn render(&self, d: &Drawing, sheet: &Sheet, budget: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
        if budget.cancel.load(Ordering::Relaxed) {
            return Err(RenderFailure::Cancelled);
        }
        let pages = sheet_pages_for_drawing(d, sheet).map_err(RenderFailure::Error)?;
        let plot = budget_plot().ok_or_else(|| RenderFailure::Error("budgeted PDF plotting is not available in this build".into()))?;
        let options = json!({"pages": pages, "output": "pdf"});
        let result = plot(d, &Space::Model, &options, budget.cancel.clone(), budget.max_prims)
            .map_err(|error| if error.contains("SHEET_TOO_LARGE") { RenderFailure::TooLarge(error) } else { RenderFailure::Error(error) });
        budget.check_result(result)
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, RenderFailure> {
        let merge = merge_pdf().ok_or_else(|| RenderFailure::Error("PDF merging is not available in this build".into()))?;
        merge(parts, labels, title).map_err(RenderFailure::Error)
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

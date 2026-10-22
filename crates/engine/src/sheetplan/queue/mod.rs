pub mod render;
pub mod runner;
pub mod store;
#[cfg(not(target_arch = "wasm32"))]
pub mod store_fs;
pub mod types;

#[cfg(not(target_arch = "wasm32"))]
pub use render::Watchdog;
pub use render::{PdfSheetRenderer, RenderFailure, SheetBudget, SheetRenderer};
pub use runner::{JobRunner, Progress, SnapshotLoader, SourceLoader, StepOutcome, run_to_end};
pub use store::{JobStore, MemoryJobStore, artifact_name};
#[cfg(not(target_arch = "wasm32"))]
pub use store_fs::DirJobStore;
pub use types::*;

#[derive(Default)]
pub(crate) struct SessionJobs {
    pub store: Option<Box<dyn JobStore>>,
    pub persisted: bool,
    pub queued: std::collections::VecDeque<PrintJob>,
    pub cancel_requests: Vec<String>,
}

impl crate::Session {
    pub fn set_job_store(&mut self, store: Box<dyn JobStore>) {
        self.plot_jobs.store = Some(store);
        self.plot_jobs.persisted = true;
    }

    pub fn job_store_mut(&mut self) -> Option<&mut (dyn JobStore + 'static)> {
        self.plot_jobs.store.as_deref_mut()
    }

    pub fn take_queued_job(&mut self) -> Option<PrintJob> {
        self.plot_jobs.queued.pop_front()
    }

    pub fn take_cancel_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut self.plot_jobs.cancel_requests)
    }

    pub(crate) fn ensure_job_store(&mut self) -> &mut (dyn JobStore + 'static) {
        self.plot_jobs.store.get_or_insert_with(|| Box::new(MemoryJobStore::default())).as_mut()
    }
}

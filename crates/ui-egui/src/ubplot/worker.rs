use std::collections::{BTreeMap, BTreeSet};

use cadcraft_engine::sheetplan::queue::{JobStatus, Progress, PublishManifest};

pub(crate) fn status_label(status: JobStatus, interrupted: bool) -> &'static str {
    if interrupted {
        return crate::tl!("Interrupted");
    }
    match status {
        JobStatus::Queued => crate::tl!("Queued"),
        JobStatus::Running => crate::tl!("Running"),
        JobStatus::Cancelled => crate::tl!("Cancelled"),
        JobStatus::Done => crate::tl!("Done"),
        JobStatus::Failed => crate::tl!("Failed"),
    }
}

fn job_notice(interrupted: &BTreeSet<String>, manifests: &BTreeMap<String, PublishManifest>, previous: &mut Option<String>) -> Option<String> {
    let current = interrupted.first();
    if previous.as_ref() == current {
        return None;
    }
    let old = std::mem::replace(previous, current.cloned());
    if let Some(id) = current {
        return Some(crate::tf!("Print job {jobId} was interrupted", jobId = id));
    }
    let id = old?;
    Some(match manifests.get(&id) {
        Some(manifest) => crate::tf!("Print job {jobId}: {status}", jobId = id, status = status_label(manifest.status, false)),
        None => crate::tf!("Print job {jobId} discarded", jobId = id),
    })
}

// The host interface passes the final manifest by value.
#[allow(clippy::large_enum_variant)]
pub enum WorkerEvent {
    Progress(Progress),
    Finished(PublishManifest),
    Failed(String),
}

pub use frame::FrameJobPump;

#[cfg(target_arch = "wasm32")]
pub use frame::FrameJobPump as JobPump;

#[cfg(target_arch = "wasm32")]
pub(crate) fn tick(app: &mut crate::CadApp, ctx: &egui::Context) {
    crate::i18n::with_language(app.language(), || {
        if let Some(notice) = app.job_pump.advance(&mut app.session, ctx) {
            app.set_status(notice);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use desktop::tick;
#[cfg(not(target_arch = "wasm32"))]
pub use desktop::{JobPump, JobWorker};

#[cfg(not(target_arch = "wasm32"))]
mod desktop {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use cadcraft_engine::sheetplan::parse_plan;
    use cadcraft_engine::sheetplan::queue::{
        DirJobStore, FolderLoader, JobRunner, JobStatus, JobStore, MAX_JOB_SHEETS, MAX_MANIFEST_BYTES, PdfSheetRenderer, PrintJob, SheetBudget,
        SheetRenderer, SnapshotLoader, SourceLoader, StepOutcome, is_folder_job,
    };

    use super::{Progress, PublishManifest, WorkerEvent, job_notice, status_label};

    pub(super) fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).ok().and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok()).unwrap_or(0)
    }

    pub struct JobWorker {
        rx: Receiver<WorkerEvent>,
        cancel: Arc<AtomicBool>,
        terminal: bool,
    }

    impl JobWorker {
        pub fn spawn(job: PrintJob, loader: Box<dyn SourceLoader + Send>, store: Box<dyn JobStore + Send>, budget: SheetBudget) -> Self {
            Self::spawn_with_renderer(job, loader, store, budget, Arc::new(PdfSheetRenderer))
        }

        pub fn spawn_with_renderer(
            job: PrintJob,
            mut loader: Box<dyn SourceLoader + Send>,
            mut store: Box<dyn JobStore + Send>,
            budget: SheetBudget,
            renderer: Arc<dyn SheetRenderer + Send + Sync>,
        ) -> Self {
            // Dropping intermediate progress keeps memory bounded when the window stops painting.
            let (tx, rx) = sync_channel(2);
            let cancel = budget.cancel.clone();
            let failure_tx = tx.clone();
            let spawned = std::thread::Builder::new().name("cadcraft-print".into()).spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<PublishManifest, String> {
                    let mut runner = JobRunner::open(job, store.as_mut(), now())?;
                    let _ = tx.try_send(WorkerEvent::Progress(runner.progress()));
                    loop {
                        let outcome = runner.step(store.as_mut(), loader.as_mut(), renderer.as_ref(), &budget, now());
                        let _ = tx.try_send(WorkerEvent::Progress(runner.progress()));
                        if matches!(outcome, StepOutcome::Finished(_)) {
                            return Ok(runner.manifest().clone());
                        }
                    }
                }));
                let event = match result {
                    Ok(Ok(manifest)) => WorkerEvent::Finished(manifest),
                    Ok(Err(error)) => WorkerEvent::Failed(error),
                    Err(_) => WorkerEvent::Failed(crate::tl!("Print worker stopped unexpectedly").into()),
                };
                let _ = tx.send(event);
            });
            if let Err(error) = spawned {
                let _ = failure_tx.try_send(WorkerEvent::Failed(error.to_string()));
            }
            Self { rx, cancel, terminal: false }
        }

        pub fn poll(&mut self) -> Vec<WorkerEvent> {
            let mut events = Vec::new();
            // A producer may refill the channel while it is drained; bound each frame's work too.
            for _ in 0..8 {
                match self.rx.try_recv() {
                    Ok(event) => {
                        self.terminal |= matches!(event, WorkerEvent::Finished(_) | WorkerEvent::Failed(_));
                        events.push(event);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if !self.terminal {
                            self.terminal = true;
                            events.push(WorkerEvent::Failed(crate::tl!("Print worker stopped unexpectedly").into()));
                        }
                        break;
                    }
                }
            }
            events
        }

        pub fn cancel(&self) {
            self.cancel.store(true, Ordering::Relaxed);
        }
    }

    impl Drop for JobWorker {
        fn drop(&mut self) {
            // The UI must never join a renderer; the worker observes cancellation between sheets.
            self.cancel();
        }
    }

    struct Pending {
        job: PrintJob,
        loader: Box<dyn SourceLoader + Send>,
    }

    pub struct JobPump {
        root: Option<PathBuf>,
        renderer: Arc<dyn SheetRenderer + Send + Sync>,
        budget: SheetBudget,
        pending: VecDeque<Pending>,
        active: Option<(String, JobWorker)>,
        pub open: bool,
        pub focus_requested: bool,
        pub manifests: BTreeMap<String, PublishManifest>,
        pub interrupted: BTreeSet<String>,
        pub errors: BTreeMap<String, String>,
        pub progress: Option<(String, Progress)>,
        notice: Option<String>,
    }

    impl Default for JobPump {
        fn default() -> Self {
            Self {
                root: None,
                renderer: Arc::new(PdfSheetRenderer),
                budget: SheetBudget::desktop(),
                pending: VecDeque::new(),
                active: None,
                open: false,
                focus_requested: false,
                manifests: BTreeMap::new(),
                interrupted: BTreeSet::new(),
                errors: BTreeMap::new(),
                progress: None,
                notice: None,
            }
        }
    }

    impl JobPump {
        pub fn new(root: PathBuf) -> Self {
            Self::with_renderer(root, Arc::new(PdfSheetRenderer), SheetBudget::desktop())
        }

        pub fn with_renderer(root: PathBuf, renderer: Arc<dyn SheetRenderer + Send + Sync>, budget: SheetBudget) -> Self {
            let mut pump = Self { root: Some(root), renderer, budget, ..Self::default() };
            let Some(store) = pump.store() else { return pump };
            for id in store.list_jobs().into_iter().take(MAX_JOB_SHEETS) {
                match read_manifest(&store, &id) {
                    Ok(manifest) => {
                        if matches!(manifest.status, JobStatus::Queued | JobStatus::Running) {
                            pump.interrupted.insert(id.clone());
                        }
                        pump.manifests.insert(id, manifest);
                    }
                    Err(error) => {
                        pump.errors.insert(id, error);
                    }
                }
            }
            pump.open = !pump.interrupted.is_empty();
            pump
        }

        pub fn busy(&self) -> bool {
            self.active.is_some() || !self.pending.is_empty()
        }

        pub fn owns(&self, id: &str) -> bool {
            self.active.as_ref().is_some_and(|(active, _)| active == id) || self.pending.iter().any(|pending| pending.job.job_id == id)
        }

        pub fn available(&self) -> bool {
            self.root.is_some()
        }

        fn store(&self) -> Option<DirJobStore> {
            self.root.clone().map(DirJobStore::new)
        }

        pub(crate) fn refresh(&mut self, id: &str) {
            let result = self
                .store()
                .ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string())
                .and_then(|store| read_manifest(&store, id));
            match result {
                Ok(manifest) => {
                    self.manifests.insert(id.into(), manifest);
                }
                Err(error) => {
                    self.errors.insert(id.into(), error);
                }
            }
        }

        pub(crate) fn resume(&mut self, session: &mut cadcraft_engine::Session, id: &str) -> Result<(), String> {
            if self.owns(id) {
                return Err(crate::tl!("Print job is already queued or running").into());
            }
            if self.pending.len() >= MAX_JOB_SHEETS {
                return Err(crate::tl!("Print job queue is full").into());
            }
            let store = self.store().ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string())?;
            let manifest = read_manifest(&store, id)?;
            if manifest.status == JobStatus::Done {
                return Ok(());
            }
            let bytes = store.read_artifact(id, "job.json")?.ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?;
            if bytes.len() > MAX_MANIFEST_BYTES {
                return Err(crate::tl!("Stored print job exceeds supported limits").into());
            }
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            let sources = value
                .get("sources")
                .and_then(serde_json::Value::as_array)
                .filter(|sources| sources.len() <= cadcraft_engine::sheetplan::queue::MAX_JOB_SOURCES)
                .ok_or_else(|| crate::tl!("Stored print job exceeds supported limits").to_string())?;
            for source in sources {
                parse_plan(source.get("plan").ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?)?;
            }
            let job: PrintJob = serde_json::from_value(value).map_err(|error| error.to_string())?;
            if job.job_id != id {
                return Err(crate::tl!("Stored print job does not match its id").into());
            }
            let loader = source_loader(session, &store, id)?;
            let mut manifest = manifest;
            manifest.status = JobStatus::Queued;
            manifest.updated_ms = now();
            let mut store = store;
            store.commit(id, &[], &manifest.to_json())?;
            self.manifests.insert(id.into(), manifest);
            self.pending.push_back(Pending { job, loader });
            self.interrupted.remove(id);
            self.errors.remove(id);
            Ok(())
        }

        pub(crate) fn discard_interrupted(&mut self, session: &mut cadcraft_engine::Session, id: &str) -> Result<(), String> {
            if self.owns(id) {
                return Err(crate::tl!("Print job is already queued or running").into());
            }
            if self.interrupted.contains(id) {
                let mut store = self.store().ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string())?;
                let mut manifest = read_manifest(&store, id)?;
                // T6 refuses removing RUNNING jobs; an interrupted job has no live writer.
                if manifest.status == JobStatus::Running {
                    manifest.status = JobStatus::Cancelled;
                    manifest.updated_ms = now();
                    store.commit(id, &[], &manifest.to_json())?;
                }
            }
            session.execute("ubplot.jobs", &serde_json::json!({"action": "remove", "jobId": id})).map_err(|error| error.to_string())?;
            self.interrupted.remove(id);
            self.manifests.remove(id);
            self.errors.remove(id);
            Ok(())
        }

        fn advance(&mut self, session: &mut cadcraft_engine::Session, ctx: &egui::Context) -> Option<String> {
            self.root.as_ref()?;
            let mut notice = None;
            for _ in 0..MAX_JOB_SHEETS {
                let Some(job) = session.take_queued_job() else { break };
                let id = job.job_id.clone();
                if self.owns(&id) {
                    continue;
                }
                self.refresh(&id);
                let loader = self
                    .store()
                    .ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string())
                    .and_then(|store| source_loader(session, &store, &id));
                match loader {
                    Ok(loader) if self.pending.len() < MAX_JOB_SHEETS => self.pending.push_back(Pending { job, loader }),
                    Ok(_) => {
                        self.errors.insert(id, crate::tl!("Print job queue is full").into());
                    }
                    Err(error) => {
                        self.errors.insert(id.clone(), error);
                        self.interrupted.insert(id);
                    }
                }
            }
            for id in session.take_cancel_requests() {
                let pending = self.pending.iter().any(|pending| pending.job.job_id == id);
                self.pending.retain(|pending| pending.job.job_id != id);
                if pending && let Some(mut store) = self.store() {
                    let result = read_manifest(&store, &id).and_then(|mut manifest| {
                        manifest.status = JobStatus::Cancelled;
                        manifest.updated_ms = now();
                        store.commit(&id, &[], &manifest.to_json())
                    });
                    if let Err(error) = result {
                        self.errors.insert(id.clone(), error);
                    }
                }
                if let Some((active, worker)) = &self.active
                    && *active == id
                {
                    worker.cancel();
                }
                self.refresh(&id);
            }
            let mut terminal = false;
            if let Some((id, worker)) = &mut self.active {
                for event in worker.poll() {
                    match event {
                        WorkerEvent::Progress(progress) => self.progress = Some((id.clone(), progress)),
                        WorkerEvent::Finished(manifest) => {
                            let status = status_label(manifest.status, false);
                            self.manifests.insert(id.clone(), manifest);
                            self.errors.remove(id);
                            let message = crate::tf!("Print job {jobId}: {status}", jobId = id, status = status);
                            session.echo(message.clone());
                            notice = Some(message);
                            terminal = true;
                        }
                        WorkerEvent::Failed(error) => {
                            self.errors.insert(id.clone(), error);
                            self.interrupted.insert(id.clone());
                            terminal = true;
                        }
                    }
                }
            }
            if terminal {
                self.active = None;
                self.progress = None;
            }
            if self.active.is_none()
                && let Some(pending) = self.pending.pop_front()
            {
                let id = pending.job.job_id.clone();
                // Mark RUNNING before spawning so T6 cancel/remove never writes over a live runner.
                let start =
                    self.store().ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string()).and_then(|mut store| {
                        let mut manifest = read_manifest(&store, &id)?;
                        if manifest.status == JobStatus::Done {
                            return Err(crate::tl!("Print job is already complete").into());
                        }
                        manifest.status = JobStatus::Running;
                        manifest.updated_ms = now();
                        store.commit(&id, &[], &manifest.to_json())?;
                        self.manifests.insert(id.clone(), manifest);
                        let budget = SheetBudget { cancel: Arc::new(AtomicBool::new(false)), ..self.budget.clone() };
                        Ok(JobWorker::spawn_with_renderer(pending.job, pending.loader, Box::new(store), budget, self.renderer.clone()))
                    });
                match start {
                    Ok(worker) => self.active = Some((id, worker)),
                    Err(error) => {
                        self.errors.insert(id.clone(), error);
                        self.interrupted.insert(id);
                    }
                }
            }
            if self.busy() {
                ctx.request_repaint_after(Duration::from_millis(16));
            }
            job_notice(&self.interrupted, &self.manifests, &mut self.notice).or(notice)
        }
    }

    fn source_loader(session: &mut cadcraft_engine::Session, store: &dyn JobStore, id: &str) -> Result<Box<dyn SourceLoader + Send>, String> {
        if is_folder_job(store, id)? { Ok(Box::new(FolderLoader::default())) } else { Ok(Box::new(snapshot(session)?)) }
    }

    fn snapshot(session: &mut cadcraft_engine::Session) -> Result<SnapshotLoader, String> {
        let hash = cadcraft_engine::cmd::ubplot::source_hash(session).map_err(|error| error.to_string())?;
        Ok(SnapshotLoader(Arc::new(session.doc().map_err(|error| error.to_string())?.clone()), hash))
    }

    fn read_manifest(store: &dyn JobStore, id: &str) -> Result<PublishManifest, String> {
        let text = store.read_manifest(id)?.ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?;
        let manifest = PublishManifest::from_json(&text)?;
        if manifest.job_id != id {
            return Err(crate::tl!("Stored print job does not match its id").into());
        }
        Ok(manifest)
    }

    pub(crate) fn tick(app: &mut crate::CadApp, ctx: &egui::Context) {
        crate::i18n::with_language(app.language(), || {
            if let Some(notice) = app.job_pump.advance(&mut app.session, ctx) {
                app.set_status(notice);
            }
        });
    }
}

mod frame {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};
    use std::rc::Rc;
    use std::sync::Arc;

    use cadcraft_engine::Session;
    use cadcraft_engine::sheetplan::codes::issue;
    use cadcraft_engine::sheetplan::queue::{
        JobRunner, JobStatus, JobStore, MAX_JOB_SHEETS, MAX_JOB_SOURCES, MAX_MANIFEST_BYTES, PdfSheetRenderer, PrintJob, SheetBudget, SheetRenderer,
        SnapshotLoader, StepOutcome,
    };
    use cadcraft_engine::sheetplan::{Severity, parse_plan};

    use super::{Progress, PublishManifest, job_notice, status_label};

    struct Pending {
        job: PrintJob,
        loader: SnapshotLoader,
    }

    pub struct FrameJobPump {
        available: bool,
        renderer: Rc<dyn SheetRenderer>,
        budget: SheetBudget,
        clock: fn() -> u64,
        last_frame: Option<u64>,
        pending: VecDeque<Pending>,
        active: Option<(JobRunner, SnapshotLoader)>,
        pub open: bool,
        pub focus_requested: bool,
        pub manifests: BTreeMap<String, PublishManifest>,
        pub interrupted: BTreeSet<String>,
        pub errors: BTreeMap<String, String>,
        pub progress: Option<(String, Progress)>,
        notice: Option<String>,
    }

    impl Default for FrameJobPump {
        fn default() -> Self {
            Self {
                available: false,
                renderer: Rc::new(PdfSheetRenderer),
                budget: SheetBudget::web(),
                clock: || 0,
                last_frame: None,
                pending: VecDeque::new(),
                active: None,
                open: false,
                focus_requested: false,
                manifests: BTreeMap::new(),
                interrupted: BTreeSet::new(),
                errors: BTreeMap::new(),
                progress: None,
                notice: None,
            }
        }
    }

    impl FrameJobPump {
        pub fn with_clock(session: &mut Session, clock: fn() -> u64) -> Self {
            Self::with_renderer(session, Rc::new(PdfSheetRenderer), SheetBudget::web(), clock)
        }

        pub fn with_renderer(session: &mut Session, renderer: Rc<dyn SheetRenderer>, budget: SheetBudget, clock: fn() -> u64) -> Self {
            let mut pump = Self { renderer, budget, clock, ..Self::default() };
            let Some(store) = session.job_store_mut() else { return pump };
            pump.available = true;
            for id in store.list_jobs().into_iter().take(MAX_JOB_SHEETS) {
                match read_manifest(store, &id) {
                    Ok(manifest) => {
                        if matches!(manifest.status, JobStatus::Queued | JobStatus::Running) {
                            pump.interrupted.insert(id.clone());
                        }
                        pump.manifests.insert(id, manifest);
                    }
                    Err(error) => {
                        pump.errors.insert(id, error);
                    }
                }
            }
            pump.open = !pump.interrupted.is_empty();
            pump
        }

        pub fn available(&self) -> bool {
            self.available
        }

        pub fn busy(&self) -> bool {
            self.active.is_some() || !self.pending.is_empty()
        }

        pub fn owns(&self, id: &str) -> bool {
            self.active.as_ref().is_some_and(|(runner, _)| runner.manifest().job_id == id)
                || self.pending.iter().any(|pending| pending.job.job_id == id)
        }

        pub fn refresh(&mut self, session: &mut Session, id: &str) {
            match store(session).and_then(|store| read_manifest(store, id)) {
                Ok(manifest) => {
                    self.manifests.insert(id.into(), manifest);
                }
                Err(error) => {
                    self.errors.insert(id.into(), error);
                }
            }
        }

        pub fn resume(&mut self, session: &mut Session, id: &str) -> Result<(), String> {
            if self.owns(id) {
                return Err(crate::tl!("Print job is already queued or running").into());
            }
            if self.pending.len() >= MAX_JOB_SHEETS {
                return Err(crate::tl!("Print job queue is full").into());
            }
            let manifest = read_manifest(store(session)?, id)?;
            if manifest.status == JobStatus::Done {
                return Ok(());
            }
            let bytes = store(session)?.read_artifact(id, "job.json")?.ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?;
            if bytes.len() > MAX_MANIFEST_BYTES {
                return Err(crate::tl!("Stored print job exceeds supported limits").into());
            }
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            let sources = value
                .get("sources")
                .and_then(serde_json::Value::as_array)
                .filter(|sources| sources.len() <= MAX_JOB_SOURCES)
                .ok_or_else(|| crate::tl!("Stored print job exceeds supported limits").to_string())?;
            for source in sources {
                parse_plan(source.get("plan").ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?)?;
            }
            let job: PrintJob = serde_json::from_value(value).map_err(|error| error.to_string())?;
            if job.job_id != id {
                return Err(crate::tl!("Stored print job does not match its id").into());
            }
            let loader = snapshot(session)?;
            if job.sources.len() != 1 || job.sources.first().is_none_or(|source| source.source_hash != loader.1) {
                return Err(crate::tl!("SOURCE_CHANGED: open the same source drawing before resuming this print job").into());
            }
            let mut manifest = manifest;
            manifest.status = JobStatus::Queued;
            manifest.updated_ms = (self.clock)();
            store(session)?.commit(id, &[], &manifest.to_json())?;
            self.manifests.insert(id.into(), manifest);
            self.pending.push_back(Pending { job, loader });
            self.interrupted.remove(id);
            self.errors.remove(id);
            Ok(())
        }

        pub fn discard_interrupted(&mut self, session: &mut Session, id: &str) -> Result<(), String> {
            if self.owns(id) {
                return Err(crate::tl!("Print job is already queued or running").into());
            }
            let mut manifest = read_manifest(store(session)?, id)?;
            if manifest.status == JobStatus::Running {
                manifest.status = JobStatus::Cancelled;
                manifest.updated_ms = (self.clock)();
                store(session)?.commit(id, &[], &manifest.to_json())?;
            }
            session.execute("ubplot.jobs", &serde_json::json!({"action": "remove", "jobId": id})).map_err(|error| error.to_string())?;
            self.interrupted.remove(id);
            self.manifests.remove(id);
            self.errors.remove(id);
            Ok(())
        }

        fn fail(&mut self, id: &str, error: String) {
            if let Some(manifest) = self.manifests.get_mut(id) {
                manifest.status = JobStatus::Failed;
                manifest.issues.push(issue(Severity::Block, "STORE_WRITE_FAILED", &error, None));
            }
            self.errors.insert(id.into(), error);
        }

        pub fn advance(&mut self, session: &mut Session, ctx: &egui::Context) -> Option<String> {
            if !self.available {
                return None;
            }
            let frame = ctx.cumulative_frame_nr();
            if self.last_frame == Some(frame) {
                return None;
            }
            self.last_frame = Some(frame);
            let mut notice = None;
            for _ in 0..MAX_JOB_SHEETS {
                let Some(job) = session.take_queued_job() else { break };
                let id = job.job_id.clone();
                if self.owns(&id) {
                    continue;
                }
                self.refresh(session, &id);
                match snapshot(session) {
                    Ok(loader) if self.pending.len() < MAX_JOB_SHEETS => self.pending.push_back(Pending { job, loader }),
                    Ok(_) => {
                        self.errors.insert(id, crate::tl!("Print job queue is full").into());
                    }
                    Err(error) => {
                        self.errors.insert(id.clone(), error);
                        self.interrupted.insert(id);
                    }
                }
            }
            for id in session.take_cancel_requests() {
                self.pending.retain(|pending| pending.job.job_id != id);
                if let Some((runner, _)) = &mut self.active
                    && runner.manifest().job_id == id
                {
                    runner.cancel();
                }
                self.refresh(session, &id);
            }
            if self.active.is_none()
                && let Some(pending) = self.pending.pop_front()
            {
                let id = pending.job.job_id.clone();
                let start = store(session).and_then(|store| {
                    let mut manifest = read_manifest(store, &id)?;
                    manifest.status = JobStatus::Running;
                    manifest.updated_ms = (self.clock)();
                    store.commit(&id, &[], &manifest.to_json())?;
                    let mut runner = JobRunner::open(pending.job, store, (self.clock)())?;
                    runner.set_frame_clock(self.clock);
                    Ok((runner, pending.loader))
                });
                match start {
                    Ok(active) => self.active = Some(active),
                    Err(error) => self.fail(&id, error),
                }
            }
            let mut terminal = false;
            if let Some((runner, loader)) = &mut self.active {
                let id = runner.manifest().job_id.clone();
                // A single step is the frame's entire rendering allowance, including retries and merges.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    store(session).map(|store| runner.step(store, loader, self.renderer.as_ref(), &self.budget, (self.clock)()))
                }));
                self.progress = Some((id.clone(), runner.progress()));
                self.manifests.insert(id.clone(), runner.manifest().clone());
                match result {
                    Ok(Ok(StepOutcome::Worked)) => {}
                    Ok(Ok(StepOutcome::Finished(_))) => {
                        terminal = true;
                        let message = crate::tf!("Print job {jobId}: {status}", jobId = id, status = status_label(runner.manifest().status, false));
                        session.echo(message.clone());
                        notice = Some(message);
                    }
                    Ok(Err(error)) => {
                        self.fail(&id, error);
                        terminal = true;
                    }
                    Err(_) => {
                        self.fail(&id, crate::tl!("Print worker stopped unexpectedly").into());
                        terminal = true;
                    }
                }
            } else if let Ok(store) = store(session) {
                match store.poll() {
                    Ok(false) => ctx.request_repaint(),
                    Err(error) => {
                        self.errors.insert("storage".into(), error);
                    }
                    Ok(true) => {}
                }
            }
            if terminal {
                self.active = None;
                self.progress = None;
            }
            if self.busy() {
                ctx.request_repaint();
            }
            job_notice(&self.interrupted, &self.manifests, &mut self.notice).or(notice)
        }
    }

    fn store(session: &mut Session) -> Result<&mut (dyn JobStore + 'static), String> {
        session.job_store_mut().ok_or_else(|| crate::tl!("Background printing is not available in this build").to_string())
    }

    fn read_manifest(store: &dyn JobStore, id: &str) -> Result<PublishManifest, String> {
        let text = store.read_manifest(id)?.ok_or_else(|| crate::tl!("Stored print job is missing").to_string())?;
        let manifest = PublishManifest::from_json(&text)?;
        if manifest.job_id != id {
            return Err(crate::tl!("Stored print job does not match its id").into());
        }
        Ok(manifest)
    }

    fn snapshot(session: &mut Session) -> Result<SnapshotLoader, String> {
        let hash = cadcraft_engine::cmd::ubplot::source_hash(session).map_err(|error| error.to_string())?;
        Ok(SnapshotLoader(Arc::new(session.doc().map_err(|error| error.to_string())?.clone()), hash))
    }
}

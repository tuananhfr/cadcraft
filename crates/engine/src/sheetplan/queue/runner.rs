use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cadcraft_doc::Drawing;

use super::render::{RenderFailure, SheetBudget, SheetRenderer};
use super::store::{JobStore, artifact_name, validate_name, validate_output_file_name};
use super::types::*;
use crate::hash::sha256_hex;
use crate::sheetplan::codes::issue;
use crate::sheetplan::{Severity, Sheet, SheetStatus};

pub trait SourceLoader {
    fn load(&mut self, index: u32, src: &JobSource) -> Result<Arc<Drawing>, String>;
}

pub struct SnapshotLoader(pub Arc<Drawing>, pub String);

impl SourceLoader for SnapshotLoader {
    fn load(&mut self, _: u32, src: &JobSource) -> Result<Arc<Drawing>, String> {
        if self.1.is_empty() {
            return Err("SOURCE_UNAVAILABLE: the drawing snapshot has no source hash".into());
        }
        if self.1 != src.source_hash {
            return Err("SOURCE_CHANGED: the drawing snapshot differs from the queued source".into());
        }
        Ok(self.0.clone())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepOutcome {
    Worked,
    Finished(JobStatus),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub done: u32,
    pub failed: u32,
    pub total: u32,
    pub files: u32,
    pub current: Option<String>,
}

struct FileGroup {
    name: String,
    sheets: Vec<(usize, u32)>,
    pages: u32,
}

struct PendingCommit {
    manifest: PublishManifest,
    artifacts: Vec<(String, Vec<u8>)>,
    completed_group: Option<String>,
    cleanup: Vec<String>,
}

pub struct JobRunner {
    job: PrintJob,
    manifest: PublishManifest,
    sheets: Vec<(usize, usize)>,
    groups: Vec<FileGroup>,
    completed_groups: BTreeSet<String>,
    pending: Option<PendingCommit>,
    awaiting: Option<PendingCommit>,
    frame_clock: Option<fn() -> u64>,
    write_failures: u8,
    cancel: Arc<AtomicBool>,
    loaded: Option<(u32, Arc<Drawing>)>,
    finished: bool,
}

fn page_count(sheet: &Sheet) -> Result<u32, String> {
    if sheet.split.as_ref().is_some_and(|split| split.pieces.len() > cadcraft_doc::MAX_PLOT_PAGES) {
        return Err("split piece count exceeds the PDF page limit".into());
    }
    let count = sheet.split.as_ref().filter(|split| split.approved).map_or(1, |split| split.pieces.len());
    if count == 0 || count > cadcraft_doc::MAX_PLOT_PAGES {
        return Err("sheet page count is outside the PDF page limits".into());
    }
    u32::try_from(count).map_err(|error| error.to_string())
}

fn safe_sheet_id(id: &str) -> String {
    let name: String = id
        .chars()
        .take(64)
        .map(|ch| if ch.is_control() || matches!(ch, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') { '-' } else { ch })
        .collect();
    if name.is_empty() { "sheet".into() } else { name }
}

fn groups(job: &PrintJob, sheets: &[(usize, usize)]) -> Result<Vec<FileGroup>, String> {
    let mut result: Vec<FileGroup> = Vec::new();
    for (index, &(source, sheet)) in sheets.iter().enumerate() {
        let sheet = job.sources.get(source).and_then(|source| source.plan.sheets.get(sheet)).ok_or("missing job sheet")?;
        let count = page_count(sheet)?;
        let new_group = job.output.mode == OutputMode::PerSheet
            || result.last().is_none_or(|group| group.pages.saturating_add(count) > job.output.pages_per_file);
        if new_group {
            let name = if job.output.mode == OutputMode::PerSheet {
                format!("{}-{:04}-{}.pdf", job.output.stem, sheet.order, safe_sheet_id(&sheet.sheet_id))
            } else {
                String::new()
            };
            result.push(FileGroup { name, sheets: Vec::new(), pages: 0 });
        }
        let group = result.last_mut().ok_or("missing output group")?;
        group.sheets.push((index, group.pages.saturating_add(1)));
        group.pages = group.pages.checked_add(count).ok_or("output page count overflow")?;
    }
    let single = result.len() == 1;
    let mut names = BTreeSet::new();
    for (index, group) in result.iter_mut().enumerate() {
        if job.output.mode == OutputMode::Merged {
            group.name = if single { format!("{}.pdf", job.output.stem) } else { format!("{}-{:03}.pdf", job.output.stem, index + 1) };
        }
        validate_output_file_name(&group.name)?;
        if !names.insert(group.name.to_lowercase()) {
            return Err("output file names collide; use distinct sheet orders or IDs".into());
        }
    }
    Ok(result)
}

fn runtime_issue(code: &str) -> bool {
    matches!(
        code,
        "RENDER_FAILED"
            | "RENDER_TIMEOUT"
            | "SHEET_TOO_LARGE"
            | "SOURCE_CHANGED"
            | "SOURCE_UNAVAILABLE"
            | "FILE_WITHHELD"
            | "MERGE_FAILED"
            | "STORE_WRITE_FAILED"
    )
}

impl JobRunner {
    pub fn open(job: PrintJob, store: &mut dyn JobStore, now_ms: u64) -> Result<JobRunner, String> {
        if job.sources.len() > MAX_JOB_SOURCES {
            return Err("job exceeds source limits".into());
        }
        let total = job.sources.iter().try_fold(0_usize, |total, source| total.checked_add(source.plan.sheets.len()));
        if total.is_none_or(|total| total > MAX_JOB_SHEETS) {
            return Err("job exceeds source or sheet limits".into());
        }
        validate_name(&job.job_id)?;
        let mut ordered = BTreeMap::new();
        for (source_index, source) in job.sources.iter().enumerate() {
            for (sheet_index, sheet) in source.plan.sheets.iter().enumerate() {
                page_count(sheet)?;
                if sheet.order == 0 || ordered.insert((source_index, sheet.order), (source_index, sheet_index)).is_some() {
                    return Err("sheet orders must be positive and unique within a source".into());
                }
            }
        }
        PrintJob::new(job.sources.clone(), job.output.clone(), job.retry_limit)
            .map_err(|issues| issues.iter().map(|issue| format!("{}: {}", issue.code, issue.message)).collect::<Vec<_>>().join("; "))?;
        let sheets: Vec<_> = ordered.into_values().collect();
        let groups = groups(&job, &sheets)?;
        let expected = PublishManifest::new(&job, now_ms);
        let mut manifest = match store.read_manifest(&job.job_id)? {
            Some(text) => PublishManifest::from_json(&text)?,
            None => expected.clone(),
        };
        manifest.sheets.sort_by_key(|sheet| (sheet.source_index, sheet.order));
        if manifest.job_id != job.job_id
            || manifest.source_hash != expected.source_hash
            || manifest.sources.len() != expected.sources.len()
            || manifest.sources.iter().zip(&expected.sources).any(|(a, b)| a.path != b.path || a.source_hash != b.source_hash || a.sheets != b.sheets)
            || manifest.sheets.len() != expected.sheets.len()
            || manifest.sheets.iter().zip(&expected.sheets).any(|(a, b)| a.sheet_key != b.sheet_key)
            || manifest.profile_id != job.profile_id
            || manifest.profile_version != job.profile_version
        {
            return Err("manifest does not match the queued job".into());
        }
        let mut delivered = BTreeSet::new();
        for file in &manifest.files {
            let group = groups.iter().find(|group| group.name == file.name).ok_or("manifest names an unknown output file")?;
            if file.pages != group.pages || !delivered.insert(file.name.clone()) {
                return Err("manifest has an invalid or duplicate output file".into());
            }
            for &(index, page) in &group.sheets {
                let sheet = manifest.sheets.get(index).ok_or("manifest is missing an output sheet")?;
                if sheet.status != SheetStatus::Done || sheet.output_file.as_deref() != Some(file.name.as_str()) || sheet.page_index != Some(page) {
                    return Err("manifest output sheets do not match the file".into());
                }
            }
        }
        for sheet in &manifest.sheets {
            if sheet.output_file.as_ref().is_some_and(|name| !delivered.contains(name)) {
                return Err("manifest sheet names an undelivered file".into());
            }
        }
        let finished = manifest.status == JobStatus::Done;
        if finished && (delivered.len() != groups.len() || manifest.sheets.iter().any(|sheet| sheet.status != SheetStatus::Done)) {
            return Err("completed manifest is missing output files".into());
        }
        if !finished {
            manifest.issues.retain(|issue| !runtime_issue(&issue.code));
            for source in &mut manifest.sources {
                source.issues.retain(|issue| !runtime_issue(&issue.code));
            }
            for sheet in &mut manifest.sheets {
                if sheet.output_file.is_some() {
                    continue;
                }
                if sheet.status == SheetStatus::Done {
                    let bytes = store.read_artifact(&job.job_id, &artifact_name(sheet.order, sheet.source_index))?;
                    if !bytes.as_ref().is_some_and(|bytes| sheet.sha256.as_ref().is_some_and(|hash| *hash == sha256_hex(bytes))) {
                        sheet.status = SheetStatus::Ready;
                        sheet.sha256 = None;
                        sheet.finished_ms = None;
                        sheet.issues.push(issue(
                            Severity::Info,
                            "ARTIFACT_REBUILT",
                            "The missing or changed artifact will be rebuilt.",
                            Some(&sheet.sheet_id),
                        ));
                    }
                }
                if sheet.status == SheetStatus::Failed {
                    sheet.status = SheetStatus::Ready;
                    sheet.attempts = 0;
                    sheet.sha256 = None;
                    sheet.finished_ms = None;
                    sheet.issues.retain(|issue| !runtime_issue(&issue.code));
                }
                if matches!(sheet.status, SheetStatus::Blocked | SheetStatus::NeedsPreview) {
                    return Err("manifest contains a sheet that is not ready to run".into());
                }
            }
            manifest.status = JobStatus::Running;
            manifest.updated_ms = now_ms;
        }
        Ok(Self {
            job,
            manifest,
            sheets,
            groups,
            completed_groups: delivered,
            pending: None,
            awaiting: None,
            frame_clock: None,
            write_failures: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            loaded: None,
            finished,
        })
    }

    /// Frame-driven hosts measure elapsed rendering without a background watchdog.
    pub fn set_frame_clock(&mut self, clock: fn() -> u64) {
        self.frame_clock = Some(clock);
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn manifest(&self) -> &PublishManifest {
        &self.manifest
    }

    pub fn progress(&self) -> Progress {
        let count = |length| u32::try_from(length).unwrap_or(u32::MAX);
        Progress {
            done: count(self.manifest.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Done).count()),
            failed: count(self.manifest.sheets.iter().filter(|sheet| sheet.status == SheetStatus::Failed).count()),
            total: count(self.manifest.sheets.len()),
            files: count(self.manifest.files.len()),
            current: self.manifest.sheets.iter().find(|sheet| sheet.status == SheetStatus::Ready).map(|sheet| sheet.sheet_key.clone()),
        }
    }

    fn sheet(&self, index: usize) -> Option<&Sheet> {
        let &(source, sheet) = self.sheets.get(index)?;
        self.job.sources.get(source)?.plan.sheets.get(sheet)
    }

    fn commit(&mut self, store: &mut dyn JobStore, pending: PendingCommit) -> StepOutcome {
        let result = serde_json::to_string(&pending.manifest)
            .map_err(|error| error.to_string())
            .and_then(|json| store.commit(&self.job.job_id, &pending.artifacts, &json));
        match result {
            Ok(()) => match store.poll() {
                Ok(true) => self.finish_commit(store, pending, Ok(())),
                Ok(false) => {
                    self.awaiting = Some(pending);
                    StepOutcome::Worked
                }
                Err(error) => self.finish_commit(store, pending, Err(error)),
            },
            Err(error) => self.finish_commit(store, pending, Err(error)),
        }
    }

    fn finish_commit(&mut self, store: &mut dyn JobStore, pending: PendingCommit, result: Result<(), String>) -> StepOutcome {
        match result {
            Ok(()) => {
                self.write_failures = 0;
                self.manifest = pending.manifest;
                if let Some(name) = pending.completed_group {
                    self.completed_groups.insert(name);
                }
                // Delivery is durable before cleanup; leftover artifacts are safe to remove on a later run.
                if !pending.cleanup.is_empty() {
                    let _ = store.delete_artifacts(&self.job.job_id, &pending.cleanup);
                }
                self.finished = matches!(self.manifest.status, JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled);
                if self.finished { StepOutcome::Finished(self.manifest.status) } else { StepOutcome::Worked }
            }
            Err(error) => {
                self.write_failures = self.write_failures.saturating_add(1);
                if self.write_failures > self.job.retry_limit {
                    self.manifest.status = JobStatus::Failed;
                    self.manifest.updated_ms = pending.manifest.updated_ms;
                    self.manifest.issues.push(issue(Severity::Block, "STORE_WRITE_FAILED", error, None));
                    self.finished = true;
                    StepOutcome::Finished(JobStatus::Failed)
                } else {
                    self.pending = Some(pending);
                    StepOutcome::Worked
                }
            }
        }
    }

    fn save(&mut self, store: &mut dyn JobStore, manifest: PublishManifest) -> StepOutcome {
        self.commit(store, PendingCommit { manifest, artifacts: Vec::new(), completed_group: None, cleanup: Vec::new() })
    }

    pub fn step(
        &mut self,
        store: &mut dyn JobStore,
        loader: &mut dyn SourceLoader,
        renderer: &dyn SheetRenderer,
        budget: &SheetBudget,
        now_ms: u64,
    ) -> StepOutcome {
        let durable = store.poll();
        if let Some(pending) = self.awaiting.take() {
            return match durable {
                Ok(true) => self.finish_commit(store, pending, Ok(())),
                Ok(false) => {
                    self.awaiting = Some(pending);
                    StepOutcome::Worked
                }
                Err(error) => self.finish_commit(store, pending, Err(error)),
            };
        }
        match durable {
            Ok(false) => return StepOutcome::Worked,
            Err(error) => {
                self.manifest.status = JobStatus::Failed;
                self.manifest.updated_ms = now_ms;
                self.manifest.issues.push(issue(Severity::Block, "STORE_WRITE_FAILED", error, None));
                self.finished = true;
                return StepOutcome::Finished(JobStatus::Failed);
            }
            Ok(true) => {}
        }
        if self.finished {
            return StepOutcome::Finished(self.manifest.status);
        }
        if let Some(mut pending) = self.pending.take() {
            pending.manifest.updated_ms = now_ms;
            if self.cancel.load(Ordering::Relaxed) || budget.cancel.load(Ordering::Relaxed) {
                pending.manifest.status = JobStatus::Cancelled;
            }
            return self.commit(store, pending);
        }
        let mut manifest = self.manifest.clone();
        manifest.updated_ms = now_ms;
        if self.cancel.load(Ordering::Relaxed) || budget.cancel.load(Ordering::Relaxed) {
            manifest.status = JobStatus::Cancelled;
            return self.save(store, manifest);
        }
        if let Some(index) = self.groups.iter().position(|group| {
            !self.completed_groups.contains(&group.name)
                && group.sheets.iter().all(|&(index, _)| {
                    self.manifest.sheets.get(index).is_some_and(|sheet| matches!(sheet.status, SheetStatus::Done | SheetStatus::Failed))
                })
        }) {
            return self.finish_file(store, renderer, manifest, index);
        }
        let Some(index) = manifest.sheets.iter().position(|sheet| sheet.status == SheetStatus::Ready) else {
            manifest.status = if manifest.sheets.iter().any(|sheet| sheet.status == SheetStatus::Failed)
                || manifest.issues.iter().any(|issue| issue.severity == Severity::Block)
            {
                JobStatus::Failed
            } else {
                JobStatus::Done
            };
            return self.save(store, manifest);
        };
        let Some(artifact) = manifest.sheets.get(index) else {
            return self.internal_error(store, manifest);
        };
        let source_index = artifact.source_index;
        let Some(source) = usize::try_from(source_index).ok().and_then(|index| self.job.sources.get(index)) else {
            return self.internal_error(store, manifest);
        };
        if self.loaded.as_ref().is_none_or(|(index, _)| *index != source_index) {
            match loader.load(source_index, source) {
                Ok(drawing) => self.loaded = Some((source_index, drawing)),
                Err(error) => {
                    let code = if error.contains("SOURCE_CHANGED") { "SOURCE_CHANGED" } else { "SOURCE_UNAVAILABLE" };
                    for sheet in &mut manifest.sheets {
                        if sheet.source_index == source_index && sheet.status != SheetStatus::Done {
                            sheet.status = SheetStatus::Failed;
                            sheet.finished_ms = Some(now_ms);
                            sheet.issues.push(issue(Severity::Block, code, &error, Some(&sheet.sheet_id)));
                        }
                    }
                    if let Some(source) = usize::try_from(source_index).ok().and_then(|index| manifest.sources.get_mut(index)) {
                        source.issues.push(issue(Severity::Block, code, error, None));
                    }
                    return self.save(store, manifest);
                }
            }
        }
        let (Some((_, drawing)), Some(sheet)) = (&self.loaded, self.sheet(index)) else {
            return self.internal_error(store, manifest);
        };
        // A timed-out attempt poisons its own flag; retries must still be able to render.
        let budget = SheetBudget { cancel: Arc::new(AtomicBool::new(false)), ..budget.clone() };
        let started = self.frame_clock.map(|clock| clock());
        let result = budget.render(renderer, drawing, sheet);
        let elapsed = started.zip(self.frame_clock).map(|(started, clock)| clock().saturating_sub(started));
        let Some(artifact) = manifest.sheets.get_mut(index) else {
            return self.internal_error(store, manifest);
        };
        artifact.attempts = artifact.attempts.saturating_add(1);
        if elapsed.is_some_and(|elapsed| elapsed > budget.time_ms) {
            artifact.issues.push(issue(
                Severity::Warn,
                "SHEET_SLOW",
                "The sheet exceeded its frame rendering time budget.",
                Some(&artifact.sheet_id),
            ));
        }
        match result {
            Ok(bytes) => {
                artifact.status = SheetStatus::Done;
                artifact.sha256 = Some(sha256_hex(&bytes));
                artifact.finished_ms = Some(now_ms);
                let name = artifact_name(artifact.order, artifact.source_index);
                self.commit(store, PendingCommit { manifest, artifacts: vec![(name, bytes)], completed_group: None, cleanup: Vec::new() })
            }
            Err(RenderFailure::Cancelled) => {
                manifest.status = JobStatus::Cancelled;
                self.save(store, manifest)
            }
            Err(error) => {
                if artifact.attempts > self.job.retry_limit {
                    let code = match &error {
                        RenderFailure::TimedOut => "RENDER_TIMEOUT",
                        RenderFailure::TooLarge(_) => "SHEET_TOO_LARGE",
                        _ => "RENDER_FAILED",
                    };
                    artifact.status = SheetStatus::Failed;
                    artifact.finished_ms = Some(now_ms);
                    artifact.issues.push(issue(Severity::Block, code, format!("{error:?}"), Some(&artifact.sheet_id)));
                }
                self.save(store, manifest)
            }
        }
    }

    fn internal_error(&mut self, store: &mut dyn JobStore, mut manifest: PublishManifest) -> StepOutcome {
        manifest.status = JobStatus::Failed;
        manifest.issues.push(issue(Severity::Block, "RENDER_FAILED", "The queued sheet is unavailable.", None));
        self.save(store, manifest)
    }

    fn merge_file(
        &self,
        store: &dyn JobStore,
        renderer: &dyn SheetRenderer,
        group: &FileGroup,
        manifest: &PublishManifest,
    ) -> Result<Vec<u8>, RenderFailure> {
        let mut parts = Vec::new();
        let mut labels = Vec::new();
        let mut total_bytes = 0_usize;
        for &(index, _) in &group.sheets {
            let artifact = manifest.sheets.get(index).ok_or_else(|| RenderFailure::Error("missing manifest sheet".into()))?;
            let name = artifact_name(artifact.order, artifact.source_index);
            let bytes = store
                .read_artifact(&self.job.job_id, &name)
                .map_err(RenderFailure::Error)?
                .ok_or_else(|| RenderFailure::Error(format!("missing artifact {name}")))?;
            if artifact.sha256.as_ref().is_none_or(|hash| *hash != sha256_hex(&bytes)) {
                return Err(RenderFailure::Error(format!("artifact checksum mismatch: {name}")));
            }
            total_bytes = total_bytes
                .checked_add(bytes.len())
                .filter(|total| *total <= cadcraft_doc::MAX_PDF_BYTES)
                .ok_or_else(|| RenderFailure::TooLarge("merged input exceeds PDF byte limit".into()))?;
            let sheet = self.sheet(index).ok_or_else(|| RenderFailure::Error("missing job sheet".into()))?;
            let count = page_count(sheet).map_err(RenderFailure::Error)?;
            if sheet.split.as_ref().is_some_and(|split| split.approved) {
                labels.extend((1..=count).map(|page| format!("{} ({page}/{count})", sheet.sheet_id)));
            } else {
                labels.push(sheet.sheet_id.clone());
            }
            parts.push(bytes);
        }
        renderer.merge(&parts.iter().map(Vec::as_slice).collect::<Vec<_>>(), &labels, &self.job.output.stem)
    }

    fn finish_file(&mut self, store: &mut dyn JobStore, renderer: &dyn SheetRenderer, mut manifest: PublishManifest, index: usize) -> StepOutcome {
        let Some(group) = self.groups.get(index) else {
            return self.internal_error(store, manifest);
        };
        let failed: Vec<_> = group
            .sheets
            .iter()
            .filter_map(|&(index, _)| manifest.sheets.get(index))
            .filter(|sheet| sheet.status == SheetStatus::Failed)
            .map(|sheet| sheet.sheet_id.clone())
            .collect();
        let mut pending =
            PendingCommit { manifest: manifest.clone(), artifacts: Vec::new(), completed_group: Some(group.name.clone()), cleanup: Vec::new() };
        if !failed.is_empty() {
            pending.manifest.issues.push(issue(
                Severity::Block,
                "FILE_WITHHELD",
                format!("{} withheld; failed sheets: {}", group.name, failed.join(", ")),
                None,
            ));
            return self.commit(store, pending);
        }
        let merged = self.merge_file(store, renderer, group, &manifest);
        let bytes = match merged {
            Ok(bytes) if bytes.len() <= cadcraft_doc::MAX_PDF_BYTES => bytes,
            result => {
                let message = match result {
                    Err(error) => format!("{} withheld; merge failed: {error:?}", group.name),
                    Ok(_) => format!("{} withheld; merged PDF exceeds byte limit", group.name),
                };
                pending.manifest.issues.push(issue(Severity::Block, "MERGE_FAILED", message, None));
                return self.commit(store, pending);
            }
        };
        let destination = match store.deliver(&self.job, &group.name, &bytes) {
            Ok(destination) => destination,
            Err(error) => {
                self.write_failures = self.write_failures.saturating_add(1);
                if self.write_failures > self.job.retry_limit {
                    manifest.status = JobStatus::Failed;
                    manifest.issues.push(issue(Severity::Block, "STORE_WRITE_FAILED", format!("{}: {error}", group.name), None));
                    return self.save(store, manifest);
                }
                return StepOutcome::Worked;
            }
        };
        manifest.files.push(OutputFile {
            name: group.name.clone(),
            pages: group.pages,
            sha256: sha256_hex(&bytes),
            delivered_to: destination,
            delivered_ms: manifest.updated_ms,
        });
        for &(index, page) in &group.sheets {
            if let Some(sheet) = manifest.sheets.get_mut(index) {
                sheet.output_file = Some(group.name.clone());
                sheet.page_index = Some(page);
                pending.cleanup.push(artifact_name(sheet.order, sheet.source_index));
            }
        }
        pending.manifest = manifest;
        self.commit(store, pending)
    }
}

pub fn run_to_end(
    job: PrintJob,
    store: &mut dyn JobStore,
    loader: &mut dyn SourceLoader,
    renderer: &dyn SheetRenderer,
    budget: &SheetBudget,
    now: &dyn Fn() -> u64,
) -> Result<PublishManifest, String> {
    if !store.poll()? {
        return Err("asynchronous print jobs require the frame pump".into());
    }
    let mut runner = JobRunner::open(job, store, now())?;
    loop {
        if let StepOutcome::Finished(_) = runner.step(store, loader, renderer, budget, now()) {
            return Ok(runner.manifest);
        }
        // A synchronous loop cannot yield to the browser to complete an IndexedDB transaction.
        if !store.poll()? {
            return Err("asynchronous print jobs require the frame pump".into());
        }
    }
}

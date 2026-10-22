use cadcraft_engine::sheetplan::queue::store::validate_output_file_name;
use cadcraft_engine::sheetplan::queue::{JobStore, MAX_JOB_SHEETS, MAX_MANIFEST_BYTES, PrintJob, PublishManifest};

const MAX_BYTES: usize = 128 << 20;
const MAX_RECORDS: usize = 50_000;

pub trait JobKv {
    fn put_many(&mut self, entries: &[(String, Vec<u8>)]) -> Result<(), String>;
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn delete(&mut self, key: &str);
    fn keys(&self, prefix: &str) -> Vec<String>;
    fn poll(&mut self) -> Result<bool, String> {
        Ok(true)
    }
}

pub struct IdbJobStore<K: JobKv> {
    kv: K,
    download: fn(&str, &str, &[u8]) -> Result<(), String>,
}

fn component(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("invalid job storage name".into());
    }
    Ok(())
}

fn manifest_key(id: &str) -> String {
    format!("{id}/manifest")
}

fn artifact_key(id: &str, name: &str) -> String {
    format!("{id}/artifacts/{name}")
}

impl<K: JobKv> IdbJobStore<K> {
    pub fn new(kv: K, download: fn(&str, &str, &[u8]) -> Result<(), String>) -> Self {
        Self { kv, download }
    }
}

impl<K: JobKv> JobStore for IdbJobStore<K> {
    fn poll(&mut self) -> Result<bool, String> {
        self.kv.poll()
    }

    fn list_jobs(&self) -> Vec<String> {
        self.kv
            .keys("")
            .into_iter()
            .take(MAX_RECORDS)
            .filter_map(|key| key.strip_suffix("/manifest").filter(|id| component(id).is_ok()).map(str::to_string))
            .take(MAX_JOB_SHEETS)
            .collect()
    }

    fn read_manifest(&self, id: &str) -> Result<Option<String>, String> {
        component(id)?;
        self.kv
            .get(&manifest_key(id))
            .map(|bytes| {
                if bytes.len() > MAX_MANIFEST_BYTES {
                    return Err("stored print manifest exceeds its byte limit".into());
                }
                String::from_utf8(bytes).map_err(|error| error.to_string())
            })
            .transpose()
    }

    fn commit(&mut self, id: &str, artifacts: &[(String, Vec<u8>)], manifest: &str) -> Result<(), String> {
        component(id)?;
        if artifacts.len() > MAX_JOB_SHEETS || manifest.len() > MAX_MANIFEST_BYTES {
            return Err("print job commit exceeds supported limits".into());
        }
        if PublishManifest::from_json(manifest)?.job_id != id {
            return Err("print manifest does not match its id".into());
        }
        let mut total = manifest.len();
        let mut names = std::collections::BTreeSet::new();
        for (name, bytes) in artifacts {
            component(name)?;
            if !names.insert(name) {
                return Err("duplicate print artifact name".into());
            }
            total = total.checked_add(bytes.len()).filter(|total| *total <= MAX_BYTES).ok_or("print job commit exceeds its byte limit")?;
        }
        let mut entries: Vec<_> = artifacts.iter().map(|(name, bytes)| (artifact_key(id, name), bytes.clone())).collect();
        entries.push((manifest_key(id), manifest.as_bytes().to_vec()));
        self.kv.put_many(&entries)
    }

    fn read_artifact(&self, id: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        component(id)?;
        component(name)?;
        let bytes = self.kv.get(&artifact_key(id, name));
        if bytes.as_ref().is_some_and(|bytes| bytes.len() > MAX_BYTES) {
            return Err("stored print artifact exceeds its byte limit".into());
        }
        Ok(bytes)
    }

    fn delete_artifacts(&mut self, id: &str, names: &[String]) -> Result<(), String> {
        component(id)?;
        if names.len() > MAX_JOB_SHEETS {
            return Err("too many print artifacts to delete".into());
        }
        for name in names {
            component(name)?;
        }
        for name in names {
            self.kv.delete(&artifact_key(id, name));
        }
        Ok(())
    }

    fn deliver(&mut self, job: &PrintJob, name: &str, bytes: &[u8]) -> Result<String, String> {
        component(&job.job_id)?;
        validate_output_file_name(name)?;
        if bytes.len() > MAX_BYTES {
            return Err("print output exceeds its byte limit".into());
        }
        (self.download)(name, "application/pdf", bytes)?;
        Ok(format!("download:{name}"))
    }

    fn remove_job(&mut self, id: &str) -> Result<(), String> {
        component(id)?;
        for key in self.kv.keys(&format!("{id}/")).into_iter().take(MAX_RECORDS) {
            self.kv.delete(&key);
        }
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::collections::{BTreeMap, VecDeque};
    use std::rc::Rc;

    use crate::idb::{IdbStore, Pending};

    use super::*;

    struct Write {
        entries: Vec<(String, Rc<Vec<u8>>)>,
        deleted: Option<String>,
        pending: Option<Pending>,
    }

    pub struct BrowserKv {
        idb: IdbStore,
        mirror: BTreeMap<String, Rc<Vec<u8>>>,
        durable: BTreeMap<String, Rc<Vec<u8>>>,
        writes: VecDeque<Write>,
        queued_bytes: usize,
        error: Option<String>,
    }

    impl BrowserKv {
        pub async fn open() -> Result<Self, String> {
            let idb = IdbStore::open().await.map_err(|error| error.message())?;
            let entries = crate::idb::with_timeout(idb.preload_print_jobs(MAX_RECORDS, MAX_BYTES)).await.map_err(|error| error.message())?;
            let mirror: BTreeMap<_, _> = entries.into_iter().map(|(key, bytes)| (key, Rc::new(bytes))).collect();
            Ok(Self { idb, durable: mirror.clone(), mirror, writes: VecDeque::new(), queued_bytes: 0, error: None })
        }

        fn check_queue(&self) -> Result<(), String> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            if self.writes.len() >= MAX_RECORDS {
                return Err("print storage queue is full".into());
            }
            Ok(())
        }
    }

    impl JobKv for BrowserKv {
        fn put_many(&mut self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
            self.check_queue()?;
            if entries.len() > MAX_RECORDS {
                return Err("too many print storage entries".into());
            }
            let added = entries
                .iter()
                .try_fold(0usize, |total, (key, bytes)| total.checked_add(key.len())?.checked_add(bytes.len()))
                .ok_or("print storage queue exceeds its byte limit")?;
            let queued_bytes =
                self.queued_bytes.checked_add(added).filter(|total| *total <= MAX_BYTES).ok_or("print storage queue exceeds its byte limit")?;
            let mut count = self.mirror.len();
            let mut total = self
                .mirror
                .iter()
                .try_fold(0usize, |total, (key, bytes)| total.checked_add(key.len())?.checked_add(bytes.len()))
                .ok_or("print storage exceeds its byte limit")?;
            for (key, bytes) in entries {
                if let Some(old) = self.mirror.get(key) {
                    total = total.saturating_sub(key.len().saturating_add(old.len()));
                } else {
                    count = count.saturating_add(1);
                }
                total =
                    total.checked_add(key.len()).and_then(|total| total.checked_add(bytes.len())).ok_or("print storage exceeds its byte limit")?;
            }
            if total > MAX_BYTES || count > MAX_RECORDS {
                return Err("print storage exceeds supported limits".into());
            }
            let entries: Vec<_> = entries.iter().map(|(key, bytes)| (key.clone(), Rc::new(bytes.clone()))).collect();
            self.mirror.extend(entries.iter().cloned());
            self.queued_bytes = queued_bytes;
            self.writes.push_back(Write { entries, deleted: None, pending: None });
            Ok(())
        }

        fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.mirror.get(key).map(|bytes| bytes.as_ref().clone())
        }

        fn delete(&mut self, key: &str) {
            if let Err(error) = self.check_queue() {
                self.error = Some(error);
                return;
            }
            self.writes.push_back(Write { entries: Vec::new(), deleted: Some(key.into()), pending: None });
            self.mirror.remove(key);
        }

        fn keys(&self, prefix: &str) -> Vec<String> {
            self.mirror.keys().filter(|key| key.starts_with(prefix)).cloned().collect()
        }

        fn poll(&mut self) -> Result<bool, String> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            // Submit the next transaction only after the previous one commits, so rollback is exact.
            for _ in 0..MAX_RECORDS {
                let Some(write) = self.writes.front_mut() else { return Ok(true) };
                if write.pending.is_none() {
                    let pending = match &write.deleted {
                        Some(key) => self.idb.delete("print-jobs", key),
                        None => {
                            let entries: Vec<_> = write.entries.iter().map(|(key, bytes)| (key.clone(), bytes.as_ref().clone())).collect();
                            self.idb.put_many("print-jobs", &entries)
                        }
                    };
                    write.pending = Some(pending);
                }
                match write.pending.as_ref().and_then(Pending::result) {
                    None => return Ok(false),
                    Some(Err(error)) => {
                        let error = error.message();
                        self.mirror = self.durable.clone();
                        self.writes.clear();
                        self.queued_bytes = 0;
                        self.error = Some(error.clone());
                        return Err(error);
                    }
                    Some(Ok(())) => {
                        if let Some(write) = self.writes.pop_front() {
                            if let Some(key) = write.deleted {
                                self.durable.remove(&key);
                            } else {
                                for (key, bytes) in write.entries {
                                    self.queued_bytes = self.queued_bytes.saturating_sub(key.len().saturating_add(bytes.len()));
                                    self.durable.insert(key, bytes);
                                }
                            }
                        }
                    }
                }
            }
            Ok(false)
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::BrowserKv;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::rc::Rc;
    use std::sync::Arc;

    use cadcraft_doc::Drawing;
    use cadcraft_engine::sheetplan::Sheet;
    use cadcraft_engine::sheetplan::queue::*;
    use serde_json::json;

    use super::*;

    thread_local! {
        static DOWNLOADS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    #[derive(Default)]
    struct State {
        data: BTreeMap<String, Vec<u8>>,
        batches: Vec<Vec<String>>,
        quota: bool,
        async_quota: bool,
        delayed: bool,
        acknowledged: bool,
        waiting: bool,
    }

    #[derive(Clone, Default)]
    struct FakeKv(Rc<RefCell<State>>);

    impl JobKv for FakeKv {
        fn put_many(&mut self, entries: &[(String, Vec<u8>)]) -> Result<(), String> {
            let mut state = self.0.borrow_mut();
            state.batches.push(entries.iter().map(|(key, _)| key.clone()).collect());
            if state.quota {
                return Err("QuotaExceededError".into());
            }
            state.data.extend(entries.iter().cloned());
            state.waiting = state.delayed;
            Ok(())
        }

        fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.0.borrow().data.get(key).cloned()
        }

        fn delete(&mut self, key: &str) {
            self.0.borrow_mut().data.remove(key);
        }

        fn keys(&self, prefix: &str) -> Vec<String> {
            self.0.borrow().data.keys().filter(|key| key.starts_with(prefix)).cloned().collect()
        }

        fn poll(&mut self) -> Result<bool, String> {
            let mut state = self.0.borrow_mut();
            if state.async_quota {
                return Err("QuotaExceededError".into());
            }
            if state.waiting && !state.acknowledged {
                return Ok(false);
            }
            state.waiting = false;
            Ok(true)
        }
    }

    fn download(name: &str, mime: &str, _: &[u8]) -> Result<(), String> {
        assert_eq!(mime, "application/pdf");
        DOWNLOADS.with(|downloads| downloads.borrow_mut().push(name.into()));
        Ok(())
    }

    struct Renderer(Rc<Cell<usize>>);

    impl SheetRenderer for Renderer {
        fn render(&self, _: &Drawing, sheet: &Sheet, _: &SheetBudget) -> Result<Vec<u8>, RenderFailure> {
            self.0.set(self.0.get() + 1);
            Ok(sheet.sheet_id.as_bytes().to_vec())
        }

        fn merge(&self, parts: &[&[u8]], _: &[String], _: &str) -> Result<Vec<u8>, RenderFailure> {
            Ok(parts.concat())
        }
    }

    fn job(count: u32) -> PrintJob {
        let sheets: Vec<_> = (1..=count)
            .map(|order| {
                json!({
                    "sheet_id": format!("S{order}"), "order": order, "mode": "window",
                    "source": {"kind": "window", "outline": [[0,0],[10,0],[10,10],[0,10]], "rotation_deg": 0},
                    "paper_mm": [210,297], "landscape": false, "plot_scale": "fit", "status": "READY"
                })
            })
            .collect();
        let plan = serde_json::from_value(json!({
            "schema_version": "1.0", "job_id": "plan", "source_hash": "same-source", "sheets": sheets
        }))
        .unwrap();
        PrintJob::new(
            vec![JobSource { path: None, source_hash: "same-source".into(), plan }],
            OutputSpec { pages_per_file: 2, ..Default::default() },
            1,
        )
        .unwrap()
    }

    fn loader(hash: &str) -> SnapshotLoader {
        SnapshotLoader(Arc::new(Drawing::default()), hash.into())
    }

    #[test]
    fn idb_store_commit_writes_artifacts_before_manifest() {
        let kv = FakeKv::default();
        let mut store = IdbJobStore::new(kv.clone(), download);
        let job = job(2);
        let manifest = PublishManifest::new(&job, 0).to_json();
        let artifacts = vec![("s00000-0001.pdf".into(), b"one".to_vec()), ("s00000-0002.pdf".into(), b"two".to_vec())];
        store.commit(&job.job_id, &artifacts, &manifest).unwrap();
        let state = kv.0.borrow();
        assert_eq!(state.batches.len(), 1);
        assert_eq!(
            state.batches.first().unwrap(),
            &vec![artifact_key(&job.job_id, "s00000-0001.pdf"), artifact_key(&job.job_id, "s00000-0002.pdf"), manifest_key(&job.job_id)],
        );
        assert_eq!(store.read_artifact(&job.job_id, "s00000-0001.pdf").unwrap(), Some(b"one".to_vec()));
        assert_eq!(store.read_manifest(&job.job_id).unwrap(), Some(manifest));
    }

    #[test]
    fn quota_error_fails_job_and_keeps_running() {
        for asynchronous in [false, true] {
            DOWNLOADS.with(|downloads| downloads.borrow_mut().clear());
            let kv = FakeKv::default();
            let mut store = IdbJobStore::new(kv.clone(), download);
            let job = job(3);
            let mut runner = JobRunner::open(job.clone(), &mut store, 0).unwrap();
            let calls = Rc::new(Cell::new(0));
            let renderer = Renderer(calls.clone());
            let mut loader = loader("same-source");
            if asynchronous {
                let mut state = kv.0.borrow_mut();
                state.delayed = true;
                state.acknowledged = false;
            } else {
                kv.0.borrow_mut().quota = true;
            }
            runner.step(&mut store, &mut loader, &renderer, &SheetBudget::web(), 0);
            if asynchronous {
                kv.0.borrow_mut().async_quota = true;
            }
            let mut finished = false;
            for _ in 0..10 {
                if runner.step(&mut store, &mut loader, &renderer, &SheetBudget::web(), 1) == StepOutcome::Finished(JobStatus::Failed) {
                    finished = true;
                    break;
                }
            }
            assert!(finished);
            assert_eq!(runner.manifest().status, JobStatus::Failed);
            assert!(runner.manifest().issues.iter().any(|issue| issue.code == "STORE_WRITE_FAILED"));
            assert_eq!(calls.get(), 1);
            DOWNLOADS.with(|downloads| assert!(downloads.borrow().is_empty()));
            let mut session = cadcraft_engine::Session::new();
            assert!(session.execute("line", &json!({"points": [[0,0],[1,1]]})).is_ok());
        }
    }

    #[test]
    fn reload_resumes_without_redelivering_files() {
        DOWNLOADS.with(|downloads| downloads.borrow_mut().clear());
        let kv = FakeKv::default();
        let mut store = IdbJobStore::new(kv.clone(), download);
        let job = job(6);
        let calls = Rc::new(Cell::new(0));
        let renderer = Renderer(calls.clone());
        let mut runner = JobRunner::open(job.clone(), &mut store, 0).unwrap();
        let mut source = loader("same-source");
        for _ in 0..20 {
            runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 1);
            if runner.manifest().files.len() == 2 {
                break;
            }
        }
        assert_eq!(runner.manifest().files.len(), 2);
        assert_eq!(calls.get(), 4);
        drop(runner);
        drop(store);
        let mut store = IdbJobStore::new(kv, download);
        let mut runner = JobRunner::open(job, &mut store, 2).unwrap();
        let mut source = loader("same-source");
        let mut finished = false;
        for _ in 0..20 {
            if runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 3) == StepOutcome::Finished(JobStatus::Done) {
                finished = true;
                break;
            }
        }
        assert!(finished);
        assert_eq!(calls.get(), 6);
        DOWNLOADS.with(|downloads| assert_eq!(&*downloads.borrow(), &["plot-001.pdf", "plot-002.pdf", "plot-003.pdf"]));
        assert!(runner.manifest().files.iter().all(|file| file.delivered_to == format!("download:{}", file.name)));
    }

    #[test]
    fn pending_transaction_blocks_rendering_and_delivery_until_acknowledged() {
        DOWNLOADS.with(|downloads| downloads.borrow_mut().clear());
        let kv = FakeKv::default();
        kv.0.borrow_mut().delayed = true;
        let mut store = IdbJobStore::new(kv.clone(), download);
        let job = job(2);
        let calls = Rc::new(Cell::new(0));
        let renderer = Renderer(calls.clone());
        let mut runner = JobRunner::open(job, &mut store, 0).unwrap();
        let mut source = loader("same-source");
        runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 0);
        for _ in 0..5 {
            assert_eq!(runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 1), StepOutcome::Worked);
        }
        assert_eq!(calls.get(), 1);
        assert_eq!(runner.progress().done, 0);
        assert_eq!(kv.0.borrow().batches.len(), 1);
        DOWNLOADS.with(|downloads| assert!(downloads.borrow().is_empty()));
        kv.0.borrow_mut().acknowledged = true;
        runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 2);
        assert_eq!(runner.progress().done, 1);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn changed_source_is_never_rendered_or_downloaded_on_resume() {
        DOWNLOADS.with(|downloads| downloads.borrow_mut().clear());
        let mut store = IdbJobStore::new(FakeKv::default(), download);
        let job = job(2);
        let calls = Rc::new(Cell::new(0));
        let renderer = Renderer(calls.clone());
        let mut runner = JobRunner::open(job, &mut store, 0).unwrap();
        let mut source = loader("different-source");
        for _ in 0..10 {
            if matches!(runner.step(&mut store, &mut source, &renderer, &SheetBudget::web(), 1), StepOutcome::Finished(_)) {
                break;
            }
        }
        assert_eq!(runner.manifest().status, JobStatus::Failed);
        assert!(runner.manifest().sheets.iter().all(|sheet| sheet.issues.iter().any(|issue| issue.code == "SOURCE_CHANGED")));
        assert_eq!(calls.get(), 0);
        DOWNLOADS.with(|downloads| assert!(downloads.borrow().is_empty()));
    }

    #[test]
    fn synchronous_resume_rejects_pending_storage_without_spinning() {
        let kv = FakeKv::default();
        let mut store = IdbJobStore::new(kv.clone(), download);
        let job = job(2);
        kv.0.borrow_mut().delayed = true;
        store.commit(&job.job_id, &[], &PublishManifest::new(&job, 0).to_json()).unwrap();
        let calls = Rc::new(Cell::new(0));
        let mut source = loader("same-source");
        let error = run_to_end(job, &mut store, &mut source, &Renderer(calls.clone()), &SheetBudget::web(), &|| 0).unwrap_err();
        assert!(error.contains("frame pump"));
        assert_eq!(calls.get(), 0);
    }
}

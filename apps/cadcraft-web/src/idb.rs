//! Browser storage primitives shared by workspace persistence and later background jobs.

use std::cell::{Cell, RefCell};
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use js_sys::{Array, Promise, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Event, IdbDatabase, IdbObjectStore, IdbRequest, IdbTransaction, IdbTransactionMode};

pub use crate::idb_profile_store::{IdbError, Pending};
use crate::idb_profile_store::{MAX_PRELOAD_RECORDS, Mirror, OwnedCommit};

pub use crate::idb_schema::{DB_NAME, DB_VERSION, STORES};
pub const IDB_OPEN_TIMEOUT_MS: u32 = 3000;
const MAX_PRELOAD_BYTES: usize = 32 << 20;

#[derive(Clone)]
pub struct IdbStore {
    db: IdbDatabase,
    _connection: Rc<Connection>,
}

struct Connection(IdbDatabase);

impl Drop for Connection {
    fn drop(&mut self) {
        self.0.close();
    }
}

fn js_error(value: JsValue) -> IdbError {
    if let Some(error) = value.dyn_ref::<web_sys::DomException>() {
        if error.name() == "QuotaExceededError" {
            return IdbError::Quota;
        }
        return IdbError::Aborted(format!("{}: {}", error.name(), error.message()));
    }
    IdbError::Unavailable(value.as_string().unwrap_or_else(|| format!("{value:?}")))
}

fn request_error(request: &IdbRequest) -> JsValue {
    request.error().ok().flatten().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("IndexedDB request failed"))
}

struct RequestCallbacks {
    request: IdbRequest,
    _success: Closure<dyn FnMut(Event)>,
    _error: Closure<dyn FnMut(Event)>,
}

impl Drop for RequestCallbacks {
    fn drop(&mut self) {
        self.request.set_onsuccess(None);
        self.request.set_onerror(None);
    }
}

pub async fn request_future(request: &IdbRequest) -> Result<JsValue, IdbError> {
    let mut callbacks = None;
    let promise = Promise::new(&mut |resolve, reject| {
        let success_request = request.clone();
        let success_reject = reject.clone();
        let success = Closure::<dyn FnMut(Event)>::new(move |_| match success_request.result() {
            Ok(value) => {
                let _ = resolve.call1(&JsValue::UNDEFINED, &value);
            }
            Err(error) => {
                let _ = success_reject.call1(&JsValue::UNDEFINED, &error);
            }
        });
        let error_request = request.clone();
        let error_reject = reject.clone();
        let error = Closure::<dyn FnMut(Event)>::new(move |_| {
            let _ = error_reject.call1(&JsValue::UNDEFINED, &request_error(&error_request));
        });
        request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
        request.set_onerror(Some(error.as_ref().unchecked_ref()));
        callbacks = Some(RequestCallbacks { request: request.clone(), _success: success, _error: error });
    });
    let result = JsFuture::from(promise).await.map_err(js_error);
    drop(callbacks);
    result
}

struct Timer {
    window: web_sys::Window,
    id: i32,
    _callback: Closure<dyn FnMut()>,
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.id);
    }
}

pub async fn with_timeout<T>(future: impl Future<Output = Result<T, IdbError>>) -> Result<T, IdbError> {
    let window = web_sys::window().ok_or_else(|| IdbError::Unavailable("no browser window".into()))?;
    let mut timer = None;
    let promise = Promise::new(&mut |resolve, reject| {
        let callback = Closure::<dyn FnMut()>::new(move || {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        match window.set_timeout_with_callback_and_timeout_and_arguments_0(callback.as_ref().unchecked_ref(), IDB_OPEN_TIMEOUT_MS as i32) {
            Ok(id) => timer = Some(Timer { window: window.clone(), id, _callback: callback }),
            Err(error) => {
                let _ = reject.call1(&JsValue::UNDEFINED, &error);
            }
        }
    });
    let mut deadline = pin!(JsFuture::from(promise));
    let mut work = pin!(future);
    let result = poll_fn(|cx| {
        if let Poll::Ready(result) = work.as_mut().poll(cx) {
            return Poll::Ready(result);
        }
        match deadline.as_mut().poll(cx) {
            Poll::Ready(_) => Poll::Ready(Err(IdbError::Unavailable("IndexedDB startup timed out".into()))),
            Poll::Pending => Poll::Pending,
        }
    })
    .await;
    drop(timer);
    result
}

struct UpgradeCallback {
    request: web_sys::IdbOpenDbRequest,
    _callback: Closure<dyn FnMut(Event)>,
}

impl Drop for UpgradeCallback {
    fn drop(&mut self) {
        self.request.set_onupgradeneeded(None);
    }
}

struct OpenAttempt {
    cancelled: Rc<Cell<bool>>,
    result: Rc<RefCell<Option<Result<IdbStore, IdbError>>>>,
}

impl Drop for OpenAttempt {
    fn drop(&mut self) {
        self.cancelled.set(true);
        if let Ok(mut result) = self.result.try_borrow_mut()
            && let Some(Ok(store)) = result.take()
        {
            store.db.close();
        }
    }
}

impl IdbStore {
    pub async fn open() -> Result<Self, IdbError> {
        // A timed-out open can still finish; keep its upgrade handler alive and close the late connection.
        let cancelled = Rc::new(Cell::new(false));
        let late = Rc::clone(&cancelled);
        let result = Rc::new(RefCell::new(None));
        let output = Rc::clone(&result);
        let attempt = OpenAttempt { cancelled: Rc::clone(&cancelled), result: Rc::clone(&result) };
        let promise = wasm_bindgen_futures::future_to_promise(async move {
            let opened = Self::open_inner().await;
            if late.get() {
                if let Ok(store) = opened {
                    store.db.close();
                }
            } else if let Ok(mut slot) = output.try_borrow_mut() {
                *slot = Some(opened);
            }
            Ok(JsValue::UNDEFINED)
        });
        let awaited = with_timeout(async { JsFuture::from(promise).await.map(|_| ()).map_err(js_error) }).await;
        awaited?;
        let opened = result
            .try_borrow_mut()
            .ok()
            .and_then(|mut slot| slot.take())
            .unwrap_or_else(|| Err(IdbError::Unavailable("IndexedDB open failed".into())));
        drop(attempt);
        opened
    }

    async fn open_inner() -> Result<Self, IdbError> {
        let factory = web_sys::window()
            .ok_or_else(|| IdbError::Unavailable("no browser window".into()))?
            .indexed_db()
            .map_err(js_error)?
            .ok_or_else(|| IdbError::Unavailable("IndexedDB is unavailable".into()))?;
        let request = factory.open_with_u32(DB_NAME, DB_VERSION).map_err(js_error)?;
        let upgrade_request = request.clone();
        let callback = Closure::<dyn FnMut(Event)>::new(move |_| {
            let upgrade = || -> Result<(), JsValue> {
                let db: IdbDatabase = upgrade_request.result()?.dyn_into()?;
                for name in crate::idb_schema::missing_stores(|name| db.object_store_names().contains(name)) {
                    db.create_object_store(name)?;
                }
                Ok(())
            };
            if let Err(error) = upgrade() {
                log::warn!("workspace: IndexedDB upgrade failed: {error:?}");
                if let Some(transaction) = upgrade_request.transaction() {
                    let _ = transaction.abort();
                }
            }
        });
        request.set_onupgradeneeded(Some(callback.as_ref().unchecked_ref()));
        let upgrade = UpgradeCallback { request: request.clone(), _callback: callback };
        let db = request_future(request.as_ref()).await?.dyn_into::<IdbDatabase>().map_err(js_error)?;
        drop(upgrade);
        let connection = Rc::new(Connection(db.clone()));
        Ok(Self { db, _connection: connection })
    }

    pub async fn preload(&self, store: &str, limit: usize) -> Result<Vec<(String, Vec<u8>)>, IdbError> {
        self.preload_bounded(store, limit.min(MAX_PRELOAD_RECORDS), MAX_PRELOAD_BYTES).await
    }

    pub async fn preload_print_jobs(&self, limit: usize, max_bytes: usize) -> Result<Vec<(String, Vec<u8>)>, IdbError> {
        self.preload_bounded("print-jobs", limit.min(50_000), max_bytes.min(512 << 20)).await
    }

    async fn preload_bounded(&self, store: &str, limit: usize, max_bytes: usize) -> Result<Vec<(String, Vec<u8>)>, IdbError> {
        let reject_truncated = store == "print-jobs";
        let transaction = self.db.transaction_with_str(store).map_err(js_error)?;
        let request = transaction.object_store(store).map_err(js_error)?.open_cursor().map_err(js_error)?;
        let records = Rc::new(RefCell::new(Vec::new()));
        let collected = Rc::clone(&records);
        let used = Rc::new(Cell::new(0usize));
        let mut callbacks = None;
        let promise = Promise::new(&mut |resolve, reject| {
            let cursor_request = request.clone();
            let success_reject = reject.clone();
            let collected = Rc::clone(&collected);
            let used = Rc::clone(&used);
            let success = Closure::<dyn FnMut(Event)>::new(move |_| {
                let read = || -> Result<(), JsValue> {
                    let value = cursor_request.result()?;
                    if value.is_null() || value.is_undefined() {
                        let _ = resolve.call0(&JsValue::UNDEFINED);
                        return Ok(());
                    }
                    let cursor: web_sys::IdbCursorWithValue = value.dyn_into()?;
                    let Ok(mut records) = collected.try_borrow_mut() else { return Err(JsValue::from_str("IndexedDB preload is busy")) };
                    if records.len() >= limit {
                        log::warn!("workspace: IndexedDB preload capped at {limit} records");
                        if reject_truncated {
                            return Err(JsValue::from_str("IndexedDB preload exceeds its record limit"));
                        }
                        let _ = resolve.call0(&JsValue::UNDEFINED);
                        return Ok(());
                    }
                    let key = cursor.key()?.as_string().ok_or_else(|| JsValue::from_str("IndexedDB key is not text"))?;
                    let bytes = cursor.value()?.dyn_into::<Uint8Array>()?;
                    let size = bytes.length() as usize;
                    let next = used.get().saturating_add(size).saturating_add(key.len());
                    if next > max_bytes {
                        return Err(JsValue::from_str("IndexedDB preload exceeds its byte limit"));
                    }
                    used.set(next);
                    records.push((key, bytes.to_vec()));
                    cursor.continue_()?;
                    Ok(())
                };
                if let Err(error) = read() {
                    let _ = success_reject.call1(&JsValue::UNDEFINED, &error);
                }
            });
            let error_request = request.clone();
            let error = Closure::<dyn FnMut(Event)>::new(move |_| {
                let _ = reject.call1(&JsValue::UNDEFINED, &request_error(&error_request));
            });
            request.set_onsuccess(Some(success.as_ref().unchecked_ref()));
            request.set_onerror(Some(error.as_ref().unchecked_ref()));
            callbacks = Some(RequestCallbacks { request: request.clone(), _success: success, _error: error });
        });
        let result = JsFuture::from(promise).await.map_err(js_error);
        drop(callbacks);
        result?;
        let records = records
            .try_borrow_mut()
            .map(|mut records| std::mem::take(&mut *records))
            .map_err(|_| IdbError::Unavailable("IndexedDB preload is busy".into()))?;
        Ok(records)
    }

    pub fn put_many(&self, store: &str, items: &[(String, Vec<u8>)]) -> Pending {
        let limit = if store == "print-jobs" { 50_000 } else { MAX_PRELOAD_RECORDS };
        self.write(&[store], |transaction| {
            let store = transaction.object_store(store).map_err(js_error)?;
            for (key, bytes) in items.iter().take(limit) {
                put(&store, key, bytes)?;
            }
            if items.len() > limit {
                return Err(IdbError::Unavailable("too many IndexedDB writes".into()));
            }
            Ok(())
        })
    }

    pub fn put_drawing(&self, name: &str, bytes: &[u8], metadata: &[u8]) -> Pending {
        self.write(&["autosave", "autosave_index"], |transaction| {
            put(&transaction.object_store("autosave").map_err(js_error)?, name, bytes)?;
            put(&transaction.object_store("autosave_index").map_err(js_error)?, name, metadata)
        })
    }

    pub fn delete_drawing(&self, name: &str) -> Pending {
        self.write(&["autosave", "autosave_index"], |transaction| {
            for store in ["autosave", "autosave_index"] {
                transaction.object_store(store).map_err(js_error)?.delete(&JsValue::from_str(name)).map_err(js_error)?;
            }
            Ok(())
        })
    }

    pub async fn read_bounded(&self, store: &str, key: &str, max_bytes: usize) -> Result<Vec<u8>, IdbError> {
        let transaction = self.db.transaction_with_str(store).map_err(js_error)?;
        let request = transaction.object_store(store).map_err(js_error)?.get(&JsValue::from_str(key)).map_err(js_error)?;
        let value = request_future(&request).await?;
        if value.is_null() || value.is_undefined() {
            return Err(IdbError::Unavailable("autosave not found".into()));
        }
        let bytes = value.dyn_into::<Uint8Array>().map_err(js_error)?;
        if bytes.length() as usize > max_bytes {
            return Err(IdbError::Unavailable("autosave exceeds its byte limit".into()));
        }
        Ok(bytes.to_vec())
    }

    pub fn delete(&self, store: &str, key: &str) -> Pending {
        self.write(&[store], |transaction| {
            transaction.object_store(store).map_err(js_error)?.delete(&JsValue::from_str(key)).map_err(js_error)?;
            Ok(())
        })
    }

    pub fn keep_once(&self, store: &str, key: &str, bytes: Vec<u8>, preserve_existing: bool) -> Pending {
        let transaction = match self.transaction(&[store]) {
            Ok(transaction) => transaction,
            Err(error) => return failed(error),
        };
        let object = match transaction.object_store(store) {
            Ok(object) => object,
            Err(error) => return failed(js_error(error)),
        };
        let request = match object.get(&JsValue::from_str(key)) {
            Ok(request) => request,
            Err(error) => return failed(js_error(error)),
        };
        let reason = Rc::new(RefCell::new(None));
        let failure = Rc::clone(&reason);
        let read_request = request.clone();
        let write_transaction = transaction.clone();
        let key = key.to_string();
        let callback = Closure::<dyn FnMut(Event)>::new(move |_| {
            let keep = || -> Result<(), IdbError> {
                let old = read_request.result().map_err(js_error)?;
                if old.is_null() || old.is_undefined() {
                    put(&object, &key, &bytes)?;
                } else if !preserve_existing {
                    let old = old.dyn_into::<Uint8Array>().map_err(js_error)?;
                    if old.length() as usize != bytes.len() || old.to_vec() != bytes {
                        return Err(IdbError::Aborted("a different damaged workspace already has this key".into()));
                    }
                }
                Ok(())
            };
            if let Err(error) = keep() {
                if let Ok(mut reason) = failure.try_borrow_mut() {
                    *reason = Some(error);
                }
                let _ = write_transaction.abort();
            }
        });
        request.set_onsuccess(Some(callback.as_ref().unchecked_ref()));
        watch_transaction(transaction, reason, Some((request, callback)), Pending::default())
    }

    pub fn evict_snapshots(&self, revisions: &[u64]) -> Pending {
        if revisions.len() > MAX_PRELOAD_RECORDS {
            return failed(IdbError::Unavailable("too many IndexedDB deletions".into()));
        }
        self.write(&["snapshots"], |transaction| {
            let store = transaction.object_store("snapshots").map_err(js_error)?;
            for revision in revisions.iter().take(MAX_PRELOAD_RECORDS).filter(|revision| **revision != 0) {
                store.delete(&JsValue::from_str(&revision.to_string())).map_err(js_error)?;
            }
            Ok(())
        })
    }

    fn write(&self, stores: &[&str], enqueue: impl FnOnce(&IdbTransaction) -> Result<(), IdbError>) -> Pending {
        let transaction = match self.transaction(stores) {
            Ok(transaction) => transaction,
            Err(error) => return failed(error),
        };
        let reason = Rc::new(RefCell::new(None));
        if let Err(error) = enqueue(&transaction) {
            if let Ok(mut reason) = reason.try_borrow_mut() {
                *reason = Some(error);
            }
            let _ = transaction.abort();
        }
        watch_transaction(transaction, reason, None, Pending::default())
    }

    fn transaction(&self, stores: &[&str]) -> Result<IdbTransaction, IdbError> {
        let names = Array::new();
        for name in stores {
            names.push(&JsValue::from_str(name));
        }
        self.db.transaction_with_str_sequence_and_mode(names.as_ref(), IdbTransactionMode::Readwrite).map_err(js_error)
    }

    pub fn commit_workspace(&self, c: OwnedCommit) -> Pending {
        if c.json.len() > cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES || c.prune.len() > MAX_PRELOAD_RECORDS {
            return failed(IdbError::Unavailable("IndexedDB write exceeds its limits".into()));
        }
        self.checked_write(Some(c.expected), move |transaction| {
            let snapshots = transaction.object_store("snapshots").map_err(js_error)?;
            put(&snapshots, &c.revision.to_string(), c.json.as_bytes())?;
            put(&transaction.object_store("workspace").map_err(js_error)?, "current", c.json.as_bytes())?;
            for revision in c.prune.iter().take(MAX_PRELOAD_RECORDS).filter(|revision| **revision != 0 && **revision != c.revision) {
                snapshots.delete(&JsValue::from_str(&revision.to_string())).map_err(js_error)?;
            }
            Ok(())
        })
    }

    pub fn import_workspace(&self, mirror: Mirror) -> Pending {
        if mirror.snapshots.len() > MAX_PRELOAD_RECORDS || mirror.damaged.len() > MAX_PRELOAD_RECORDS || mirror.meta.len() > MAX_PRELOAD_RECORDS {
            return failed(IdbError::Unavailable("IndexedDB write exceeds its limits".into()));
        }
        self.checked_write(None, move |transaction| {
            if let Some(current) = &mirror.current {
                put(&transaction.object_store("workspace").map_err(js_error)?, "current", current.as_bytes())?;
            }
            let snapshots = transaction.object_store("snapshots").map_err(js_error)?;
            for (revision, json) in &mirror.snapshots {
                put(&snapshots, &revision.to_string(), json.as_bytes())?;
            }
            let damaged = transaction.object_store("damaged").map_err(js_error)?;
            for (id, raw) in &mirror.damaged {
                put(&damaged, &id.to_string(), raw)?;
            }
            let meta = transaction.object_store("meta").map_err(js_error)?;
            for (key, value) in &mirror.meta {
                put(&meta, key, value.as_bytes())?;
            }
            Ok(())
        })
    }

    fn checked_write(&self, expected: Option<u64>, enqueue: impl Fn(&IdbTransaction) -> Result<(), IdbError> + 'static) -> Pending {
        let transaction = match self.transaction(&["workspace", "snapshots", "damaged", "meta"]) {
            Ok(transaction) => transaction,
            Err(error) => return failed(error),
        };
        let request = match transaction.object_store("workspace").and_then(|store| store.get(&JsValue::from_str("current"))) {
            Ok(request) => request,
            Err(error) => {
                let _ = transaction.abort();
                return failed(js_error(error));
            }
        };
        let reason = Rc::new(RefCell::new(None));
        let pending = Pending::default();
        let observed = pending.clone();
        let failure = Rc::clone(&reason);
        let check_request = request.clone();
        let check_transaction = transaction.clone();
        let callback = Closure::<dyn FnMut(Event)>::new(move |_| {
            // IDB only permits the replacement while this read callback's transaction is active.
            let write = || -> Result<(), IdbError> {
                let current = check_request.result().map_err(js_error)?;
                let present = !current.is_null() && !current.is_undefined();
                let mut stored_json = None;
                let revision = if present {
                    let bytes = current.dyn_into::<Uint8Array>().map_err(js_error)?;
                    if bytes.length() as usize > cadcraft_ui_egui::wsprofile::store::MAX_SET_BYTES {
                        return Err(IdbError::Aborted("stored workspace exceeds its byte limit".into()));
                    }
                    stored_json = String::from_utf8(bytes.to_vec()).ok();
                    stored_json.as_deref().and_then(cadcraft_ui_egui::wsprofile::startup::stored_revision).unwrap_or(0)
                } else {
                    0
                };
                if expected.is_some_and(|expected| revision > expected) || (expected.is_none() && present) {
                    if let Some(json) = stored_json {
                        observed.observe_current(json);
                    }
                    return Err(IdbError::Conflict { stored: revision });
                }
                enqueue(&check_transaction)
            };
            if let Err(error) = write() {
                if let Ok(mut failure) = failure.try_borrow_mut() {
                    *failure = Some(error);
                }
                let _ = check_transaction.abort();
            }
        });
        request.set_onsuccess(Some(callback.as_ref().unchecked_ref()));
        watch_transaction(transaction, reason, Some((request, callback)), pending)
    }
}

fn put(store: &IdbObjectStore, key: &str, bytes: &[u8]) -> Result<(), IdbError> {
    store.put_with_key(Uint8Array::from(bytes).as_ref(), &JsValue::from_str(key)).map_err(js_error)?;
    Ok(())
}

fn failed(error: IdbError) -> Pending {
    let pending = Pending::default();
    pending.complete(Err(error));
    pending
}

fn watch_transaction(
    transaction: IdbTransaction,
    reason: Rc<RefCell<Option<IdbError>>>,
    request: Option<(IdbRequest, Closure<dyn FnMut(Event)>)>,
    pending: Pending,
) -> Pending {
    let output = pending.clone();
    let mut handlers = None;
    let promise = Promise::new(&mut |resolve, reject| {
        let complete = Closure::<dyn FnMut(Event)>::new(move |_| {
            let _ = resolve.call0(&JsValue::UNDEFINED);
        });
        let failure_transaction = transaction.clone();
        let abort = Closure::<dyn FnMut(Event)>::new(move |_| {
            let error = failure_transaction.error().map(JsValue::from).unwrap_or_else(|| JsValue::from_str("IndexedDB transaction aborted"));
            let _ = reject.call1(&JsValue::UNDEFINED, &error);
        });
        // Request errors bubble before abort; wait for abort so a failed write has fully rolled back.
        let error_transaction = transaction.clone();
        let error_reason = Rc::clone(&reason);
        let error = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let request_error = event.target().and_then(|target| target.dyn_into::<IdbRequest>().ok()).map(|request| request_error(&request));
            let error = request_error.or_else(|| error_transaction.error().map(JsValue::from));
            if let Some(error) = error
                && let Ok(mut reason) = error_reason.try_borrow_mut()
                && reason.is_none()
            {
                *reason = Some(js_error(error));
            }
        });
        transaction.set_oncomplete(Some(complete.as_ref().unchecked_ref()));
        transaction.set_onabort(Some(abort.as_ref().unchecked_ref()));
        transaction.set_onerror(Some(error.as_ref().unchecked_ref()));
        handlers = Some((complete, abort, error));
    });
    wasm_bindgen_futures::spawn_local(async move {
        let result = JsFuture::from(promise).await.map(|_| ()).map_err(js_error);
        let explicit = reason.try_borrow_mut().ok().and_then(|mut reason| reason.take());
        output.complete(explicit.map_or(result, Err));
        transaction.set_oncomplete(None);
        transaction.set_onabort(None);
        transaction.set_onerror(None);
        if let Some((request, _callback)) = request {
            request.set_onsuccess(None);
        }
        drop(handlers);
    });
    pending
}

pub async fn wait_pending(pending: &Pending) -> Result<(), IdbError> {
    with_timeout(pending.wait()).await
}

pub async fn request_persistence() -> bool {
    let Some(window) = web_sys::window() else { return false };
    let Ok(promise) = window.navigator().storage().persist() else { return false };
    with_timeout(async { JsFuture::from(promise).await.map_err(js_error) }).await.ok().and_then(|value| value.as_bool()).unwrap_or(false)
}

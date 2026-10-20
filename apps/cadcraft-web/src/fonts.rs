//! Fonts on the web: Fonts ▸ Add font file... shows the browser's file picker (a hidden
//! `<input type="file">`), and the fonts the user added are kept in `localStorage` so they are there
//! at the next visit ([`crate::font_index`] says how much). Nothing is uploaded: the page reads the
//! file in the browser.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cadcraft_fonts::ttf;
use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::fontmgr::FontChange;
use wasm_bindgen::JsCast as _;
use web_sys::js_sys;

use crate::font_index::{Entry, Index};

const INPUT_ID: &str = "cadcraft_font_input";
const ACCEPT: &str = ".ttf,.otf,.ttc,.otc";
/// localStorage: the index of kept fonts, and one entry per font (its bytes as base64).
const INDEX_KEY: &str = "cadcraft.fonts";
const FONT_KEY: &str = "cadcraft.font:";

type Picked = Result<(String, Vec<u8>), String>;

#[derive(Clone)]
pub struct Fonts {
    files: Rc<Cell<Vec<Picked>>>,
    ctx: egui::Context,
    /// Installed fonts the browser listed (name, its handle), and the names it just listed.
    local: Rc<RefCell<Vec<(String, wasm_bindgen::JsValue)>>>,
    listed: Rc<Cell<Option<Vec<String>>>>,
}

impl Fonts {
    pub fn new(ctx: &egui::Context) -> Self {
        Fonts { files: Rc::default(), ctx: ctx.clone(), local: Rc::default(), listed: Rc::default() }
    }

    /// Add the fonts kept from earlier visits. One that can't be read is forgotten.
    pub fn load(&self, app: &mut CadApp) {
        let Some(storage) = storage() else { return };
        let mut index = read_index(&storage);
        let mut lost = Vec::new();
        for e in index.entries() {
            let bytes = storage.get_item(&format!("{FONT_KEY}{}", e.stem)).ok().flatten().and_then(|s| decode(&s));
            if bytes.is_none_or(|b| app.restore_font(&e.name, b).is_err()) {
                lost.push(e.stem.clone());
            }
        }
        for stem in &lost {
            index.remove(stem);
            let _ = storage.remove_item(&format!("{FONT_KEY}{stem}"));
        }
        if !lost.is_empty() {
            let _ = storage.set_item(INDEX_KEY, &index.to_json());
        }
    }

    /// Per frame: show the picker when the dialog asked for it, add what it returned, and keep
    /// what was added or removed.
    pub fn frame(&self, app: &mut CadApp) {
        if app.take_font_pick_request()
            && let Err(e) = self.show_picker()
        {
            app.font_add_failed(format!("The browser's file picker is not available ({e})"));
        }
        if app.take_local_fonts_request() {
            self.list_local();
        }
        if let Some(names) = self.listed.take() {
            app.set_local_fonts(names);
        }
        for name in app.take_local_font_choices() {
            self.load_local(name);
        }
        for picked in self.files.take() {
            match picked {
                Ok((name, bytes)) => {
                    if let Err(e) = app.add_font(&name, bytes) {
                        app.font_add_failed(e);
                    }
                }
                Err(e) => app.font_add_failed(e),
            }
        }
        for change in app.take_font_changes() {
            match change {
                FontChange::Added { file_name, bytes } => {
                    if let Err(why) = keep(&file_name, &bytes) {
                        app.font_add_failed(format!("{why} It works for this visit only; the desktop version keeps any font."));
                    }
                }
                FontChange::Removed { stem } => forget(&stem),
            }
        }
    }

    fn show_picker(&self) -> Result<(), String> {
        let document = web_sys::window().and_then(|w| w.document()).ok_or("no browser document")?;
        let body = document.body().ok_or("the page has no body")?;
        if let Some(old) = document.get_element_by_id(INPUT_ID) {
            old.remove();
        }
        let input = document
            .create_element("input")
            .map_err(js_error)?
            .dyn_into::<web_sys::HtmlInputElement>()
            .map_err(|_| "could not create a file input".to_string())?;
        input.set_type("file");
        input.set_accept(ACCEPT);
        input.set_id(INPUT_ID);
        input.set_attribute("style", "display:none").map_err(js_error)?;
        let (fonts, chosen) = (self.clone(), input.clone());
        // Called once, when a file was chosen (browsers don't fire `change` on cancel).
        let on_change = wasm_bindgen::closure::Closure::once_into_js(move || {
            if let Some(file) = chosen.files().and_then(|f| f.get(0)) {
                fonts.read(file);
            }
            chosen.remove();
        });
        input.set_onchange(Some(on_change.unchecked_ref()));
        body.append_child(&input).map_err(js_error)?;
        input.click();
        Ok(())
    }

    fn read(&self, file: web_sys::File) {
        let name = file.name();
        let size = file.size();
        if !size.is_finite() || size < 0.0 || size > ttf::MAX_USER_FONT_BYTES as f64 {
            return self.push(Err(format!("{name} is too large to be a font.")));
        }
        let fonts = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let r = match wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                Ok(buf) => Ok((name, js_sys::Uint8Array::new(&buf).to_vec())),
                Err(e) => Err(format!("Could not read {name}: {}", js_error(e))),
            };
            fonts.push(r);
        });
    }

    fn push(&self, picked: Picked) {
        let mut files = self.files.take();
        files.push(picked);
        self.files.set(files);
        self.ctx.request_repaint();
    }
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

fn read_index(storage: &web_sys::Storage) -> Index {
    Index::parse(&storage.get_item(INDEX_KEY).ok().flatten().unwrap_or_default())
}

/// Keep a font for the next visit, or say why it can't be kept.
fn keep(file_name: &str, bytes: &[u8]) -> Result<(), String> {
    let storage = storage().ok_or("The browser does not allow storing fonts.")?;
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name).to_string();
    let stem = ttf::key(&name);
    let mut index = read_index(&storage);
    if let Some(why) = index.refusal(&stem, bytes.len()) {
        return Err(why);
    }
    let full = || "The browser has no room left to store it.".to_string();
    let key = format!("{FONT_KEY}{stem}");
    storage.set_item(&key, &encode(bytes)).map_err(|_| full())?;
    index.insert(Entry { name, stem, size: bytes.len() });
    storage.set_item(INDEX_KEY, &index.to_json()).map_err(|_| {
        let _ = storage.remove_item(&key);
        full()
    })
}

fn forget(stem: &str) {
    let Some(storage) = storage() else { return };
    let mut index = read_index(&storage);
    if index.remove(stem).is_some() {
        let _ = storage.set_item(INDEX_KEY, &index.to_json());
    }
    let _ = storage.remove_item(&format!("{FONT_KEY}{stem}"));
}

/// Bytes as base64 through the browser's `btoa`, which takes one char per byte.
fn encode(bytes: &[u8]) -> String {
    let binary: String = bytes.iter().map(|&b| char::from(b)).collect();
    web_sys::window().and_then(|w| w.btoa(&binary).ok()).unwrap_or_default()
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let binary = web_sys::window()?.atob(text).ok()?;
    binary.chars().map(|c| u8::try_from(u32::from(c)).ok()).collect()
}

fn js_error(e: wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}

/// Installed fonts, through Chromium's `queryLocalFonts` (the browser asks the user first). The
/// font handles stay here; the app only sees their names.
impl Fonts {
    /// Whether this browser can list installed fonts.
    pub fn can_list_local() -> bool {
        web_sys::window().is_some_and(|w| js_sys::Reflect::has(&w, &"queryLocalFonts".into()).unwrap_or(false))
    }

    fn list_local(&self) {
        let fonts = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match fonts.query_local().await {
                Ok(names) => {
                    fonts.listed.set(Some(names));
                    fonts.ctx.request_repaint();
                }
                Err(e) => fonts.push(Err(e)),
            }
        });
    }

    async fn query_local(&self) -> Result<Vec<String>, String> {
        let window = web_sys::window().ok_or("no browser window")?;
        let query = js_sys::Reflect::get(&window, &"queryLocalFonts".into())
            .ok()
            .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
            .ok_or("This browser cannot list installed fonts.")?;
        let promise =
            query.call0(&window).map_err(js_error)?.dyn_into::<js_sys::Promise>().map_err(|_| "This browser cannot list installed fonts.")?;
        let list = wasm_bindgen_futures::JsFuture::from(promise).await.map_err(|_| "Access to the installed fonts was not allowed.".to_string())?;
        let mut found: Vec<(String, wasm_bindgen::JsValue)> = js_sys::Array::from(&list)
            .iter()
            .filter_map(|data| Some((js_sys::Reflect::get(&data, &"fullName".into()).ok()?.as_string()?, data)))
            .collect();
        found.sort_by_key(|(name, _)| name.to_lowercase());
        found.dedup_by(|a, b| a.0 == b.0);
        let names = found.iter().map(|(n, _)| n.clone()).collect();
        *self.local.borrow_mut() = found;
        Ok(names)
    }

    /// Read an installed font the user chose; it arrives like a picked file.
    fn load_local(&self, name: String) {
        let Some(data) = self.local.borrow().iter().find(|(n, _)| *n == name).map(|(_, d)| d.clone()) else { return };
        let fonts = self.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let r = read_local(&data).await.map(|bytes| (format!("{name}.ttf"), bytes)).map_err(|e| format!("Could not read {name}: {e}"));
            fonts.push(r);
        });
    }
}

async fn read_local(data: &wasm_bindgen::JsValue) -> Result<Vec<u8>, String> {
    let blob =
        js_sys::Reflect::get(data, &"blob".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()).ok_or("the browser gave no font data")?;
    let promise = blob.call0(data).map_err(js_error)?.dyn_into::<js_sys::Promise>().map_err(|_| "the browser gave no font data")?;
    let blob = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_error)?
        .dyn_into::<web_sys::Blob>()
        .map_err(|_| "the browser gave no font data")?;
    if blob.size() > ttf::MAX_USER_FONT_BYTES as f64 {
        return Err("the font is too large".into());
    }
    let buf = wasm_bindgen_futures::JsFuture::from(blob.array_buffer()).await.map_err(js_error)?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

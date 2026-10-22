use std::cell::Cell;
use std::rc::Rc;

use cadcraft_ui_egui::fullscreen::FullscreenHost;
use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};

#[derive(Clone, Default)]
pub struct WebFullscreen {
    events: Rc<Cell<Option<bool>>>,
    installed: bool,
}

impl WebFullscreen {
    pub(crate) fn take_event(&self) -> Option<bool> {
        self.events.take()
    }
}

impl FullscreenHost for WebFullscreen {
    fn request(&self) -> Result<(), String> {
        let document = web_sys::window().and_then(|window| window.document()).ok_or("document unavailable")?;
        if !self.installed || !document.fullscreen_enabled() {
            return Err("fullscreen unavailable".into());
        }
        let element = document.document_element().ok_or("document element unavailable")?;
        self.events.set(None);
        element.request_fullscreen().map_err(|error| format!("{error:?}"))
    }

    fn exit(&self) {
        if let Some(document) = web_sys::window().and_then(|window| window.document())
            && document.fullscreen_element().is_some()
        {
            document.exit_fullscreen();
        }
    }

    fn is_active(&self) -> bool {
        web_sys::window().and_then(|window| window.document()).is_some_and(|document| document.fullscreen_element().is_some())
    }
}

pub fn install(shell: &crate::web::Shell) -> Result<WebFullscreen, JsValue> {
    let document = web_sys::window().and_then(|window| window.document()).ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let events = Rc::new(Cell::new(None));
    let on_change = {
        let document = document.clone();
        let events = Rc::clone(&events);
        let ctx = shell.7.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            events.set(Some(document.fullscreen_element().is_some()));
            ctx.request_repaint();
        })
    };
    let on_error = {
        let events = Rc::clone(&events);
        let ctx = shell.7.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            events.set(Some(false));
            ctx.request_repaint();
        })
    };
    document.add_event_listener_with_callback("fullscreenchange", on_change.as_ref().unchecked_ref())?;
    if let Err(error) = document.add_event_listener_with_callback("fullscreenerror", on_error.as_ref().unchecked_ref()) {
        let _ = document.remove_event_listener_with_callback("fullscreenchange", on_change.as_ref().unchecked_ref());
        return Err(error);
    }
    on_change.forget();
    on_error.forget();
    Ok(WebFullscreen { events, installed: true })
}

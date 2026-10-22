use std::cell::Cell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;

pub struct LeaveGuard {
    dirty: Rc<Cell<bool>>,
}

impl LeaveGuard {
    pub fn install() -> Option<LeaveGuard> {
        let window = web_sys::window()?;
        let dirty = Rc::new(Cell::new(false));
        let on_leave = {
            let dirty = Rc::clone(&dirty);
            Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |event: web_sys::BeforeUnloadEvent| {
                if dirty.get() {
                    event.prevent_default();
                    event.set_return_value("");
                }
            })
        };
        window.add_event_listener_with_callback("beforeunload", on_leave.as_ref().unchecked_ref()).ok()?;
        // The listener must remain callable for the lifetime of the page.
        on_leave.forget();
        Some(LeaveGuard { dirty })
    }

    pub fn set(&self, dirty: bool) {
        self.dirty.set(dirty);
    }
}

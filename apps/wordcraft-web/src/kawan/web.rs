//! KawanEditor bridge transport: `postMessage` between the engine iframe and its parent page.
//! Active only with `?host=kawan` inside a frame; only the parent window on the page's own origin
//! is heard or answered. The protocol itself lives in `protocol.rs`.
//!
//! Canonical copy: kawan-editor `bridge/src/kawan/web.rs`. Engine forks hold a verbatim copy in
//! `apps/<x>craft-web/src/kawan/`; edit the canonical file and run `scripts/sync-bridge.sh`.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

use super::protocol::{Host, OutFile, Post, Protocol};

/// Files the app saved or downloaded through its own UI; they leave on the next `tick`.
pub type Outbox = Rc<RefCell<Vec<OutFile>>>;

type Inbox = Rc<RefCell<Vec<(Value, Option<Vec<u8>>)>>>;
type Repaint = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub struct Bridge {
    parent: web_sys::Window,
    origin: String,
    protocol: RefCell<Protocol>,
    inbox: Inbox,
    outbox: Outbox,
    repaint: Repaint,
}

/// `?host=kawan` is in the page URL.
pub fn is_kawan_url() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|s| web_sys::UrlSearchParams::new_with_str(&s).ok())
        .and_then(|p| p.get("host"))
        .is_some_and(|h| h == "kawan")
}

impl Bridge {
    /// Some only when the URL has `?host=kawan` and the page is framed. Create it before the app
    /// starts (so a failed start can be reported), then `set_repaint` once the app has a context.
    pub fn from_url() -> Option<Bridge> {
        if !is_kawan_url() {
            return None;
        }
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        let inbox: Inbox = Rc::default();
        let repaint: Repaint = Rc::default();
        let listener = {
            let (parent, origin, inbox, repaint) = (parent.clone(), origin.clone(), inbox.clone(), repaint.clone());
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                    return;
                }
                if let Some(message) = read_message(&event.data()) {
                    inbox.borrow_mut().push(message);
                    if let Some(repaint) = &*repaint.borrow() {
                        repaint();
                    }
                }
            })
        };
        if window.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref()).is_err() {
            return None;
        }
        // The listener lives as long as the page.
        listener.forget();
        Some(Bridge { parent, origin, protocol: RefCell::new(Protocol::new()), inbox, outbox: Rc::default(), repaint })
    }

    /// Wake the app when a message arrives (it handles messages on its next frame).
    pub fn set_repaint(&self, repaint: Box<dyn Fn()>) {
        *self.repaint.borrow_mut() = Some(repaint);
    }

    /// Hand this to the app's download/save hook.
    pub fn outbox(&self) -> Outbox {
        self.outbox.clone()
    }

    pub fn post_ready(&self, engine: &str, version: &str) {
        self.post(Protocol::ready(engine, version, option_env!("KAWAN_UPSTREAM_SHA").unwrap_or("unknown")));
    }

    /// The app could not start (eframe `WebRunner::start` failed).
    pub fn post_start_failed(&self, error: &str) {
        self.post(Protocol::start_failed(error));
    }

    /// Call once per frame after the app's logic.
    pub fn tick(&self, host: &mut dyn Host) {
        let messages = std::mem::take(&mut *self.inbox.borrow_mut());
        let mut protocol = self.protocol.borrow_mut();
        for (message, bytes) in messages {
            for post in protocol.handle(&message, bytes, host) {
                self.post(post);
            }
        }
        let outbox = std::mem::take(&mut *self.outbox.borrow_mut());
        for post in protocol.tick(js_sys::Date::now(), outbox, host) {
            self.post(post);
        }
        // Look up the document only when a title is pending (the chain is lazy).
        if let Some(title) = protocol.take_title()
            && let Some(document) = web_sys::window().and_then(|w| w.document())
        {
            document.set_title(&title);
        }
    }

    fn post(&self, post: Post) {
        let Some(message) = build_message(post) else { return };
        let (message, transfer) = message;
        let _ = self.parent.post_message_with_transfer(&message, &self.origin, &transfer);
    }
}

/// The message as JSON (without `payload.bytes`) plus those bytes, if any.
fn read_message(data: &JsValue) -> Option<(Value, Option<Vec<u8>>)> {
    let payload = js_sys::Reflect::get(data, &JsValue::from_str("payload")).ok().filter(|p| p.is_object());
    let bytes = payload.as_ref().and_then(|p| {
        let value = js_sys::Reflect::get(p, &JsValue::from_str("bytes")).ok()?;
        let bytes = if let Some(array) = value.dyn_ref::<js_sys::Uint8Array>() {
            array.to_vec()
        } else if value.is_instance_of::<js_sys::ArrayBuffer>() {
            js_sys::Uint8Array::new(&value).to_vec()
        } else {
            return None;
        };
        // The event data is our own structured clone: drop the bytes before stringifying it.
        let _ = js_sys::Reflect::delete_property(p.unchecked_ref::<js_sys::Object>(), &JsValue::from_str("bytes"));
        Some(bytes)
    });
    let text = js_sys::JSON::stringify(data).ok()?.as_string()?;
    Some((serde_json::from_str(&text).ok()?, bytes))
}

fn build_message(post: Post) -> Option<(js_sys::Object, js_sys::Array)> {
    let set = |target: &JsValue, key: &str, value: &JsValue| js_sys::Reflect::set(target, &JsValue::from_str(key), value).ok();
    let message = js_sys::Object::new();
    set(&message, "kawan", &JsValue::from_f64(1.0))?;
    set(&message, "type", &JsValue::from_str(post.kind))?;
    if let Some(id) = post.id {
        // Ids come from the SDK's counter; far below 2^53.
        set(&message, "id", &JsValue::from_f64(id as f64))?;
    }
    let payload = js_sys::JSON::parse(&serde_json::to_string(&post.payload).ok()?).ok()?;
    let transfer = js_sys::Array::new();
    if let Some(bytes) = post.bytes {
        let buffer = js_sys::Uint8Array::from(bytes.as_slice()).buffer();
        set(&payload, "bytes", &buffer)?;
        transfer.push(&buffer);
    }
    set(&message, "payload", &payload)?;
    Some((message, transfer))
}

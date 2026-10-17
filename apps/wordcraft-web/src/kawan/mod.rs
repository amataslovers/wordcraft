//! KawanEditor host bridge (`?host=kawan`): the page embedding WordCraft drives it through the
//! `kawan:*` postMessage protocol. `protocol.rs` and `web.rs` are verbatim copies of the
//! kawan-editor canonical bridge; `host.rs` adapts WordCraft to it.

pub mod host;
pub mod protocol;
pub mod web;

use wordcraft_ui_egui::Services;

/// Route the app's downloads (Save, Save As, exports) through the bridge, wake the app when the
/// page sends a message, and tell the page the bridge is listening.
pub fn attach(services: &mut Services, bridge: &web::Bridge, ctx: &egui::Context) {
    let outbox = bridge.outbox();
    services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
        let name = std::path::Path::new(name).file_name().map_or_else(|| name.to_string(), |n| n.to_string_lossy().to_string());
        outbox.borrow_mut().push(protocol::OutFile { name, bytes: bytes.to_vec(), export: false });
        Ok(())
    }));
    let ctx = ctx.clone();
    bridge.set_repaint(Box::new(move || ctx.request_repaint()));
    bridge.post_ready("word", env!("CARGO_PKG_VERSION"));
}

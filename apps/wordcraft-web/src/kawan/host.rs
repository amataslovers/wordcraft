//! WordCraft as a KawanEditor bridge host.

use serde_json::{Value, json};
use wordcraft_engine::cmd::insert::base64_encode;
use wordcraft_ui_egui::WordApp;

use super::policy::{self, Policy};
use super::protocol::{Classify, CommandInfo, Host};

pub struct WordHost<'a>(pub &'a mut WordApp);

impl WordHost<'_> {
    /// What Save writes: the open file's own Word format (a .docm keeps its macros), otherwise .docx.
    fn save_name(&self) -> String {
        let ext = self.0.session.path.as_ref().and_then(|p| p.extension()).map(|e| e.to_string_lossy().to_string());
        match ext {
            Some(e) if wordcraft_engine::io::is_word_package(&e) => format!("{}.{e}", self.0.title_stem()),
            _ => format!("{}.docx", self.0.title_stem()),
        }
    }

    fn serialize(&self) -> Result<(String, Vec<u8>), String> {
        let name = self.save_name();
        // ponytail: no password or export options; encrypted documents save unencrypted until the
        // bridge passes them through.
        let bytes = wordcraft_engine::io::save_bytes(&name, &self.0.session.doc)?;
        Ok((name, bytes))
    }
}

impl Host for WordHost<'_> {
    fn open(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        // `execute`, not `run`: never the Save Changes prompt for the blank start document.
        // The bridge's own open: the host's command deny list does not apply to it.
        policy::as_host(|| self.0.execute("file.open", json!({"path": name, "data": base64_encode(&bytes)})))?;
        self.0.ui.backstage = false;
        Ok(())
    }

    fn save(&mut self) -> Result<(String, Vec<u8>), String> {
        let saved = self.serialize()?;
        self.0.session.dirty = false;
        Ok(saved)
    }

    fn export_copy(&mut self) -> Result<(String, Vec<u8>), String> {
        self.serialize()
    }

    fn exec(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.0.execute(id, params)
    }

    fn commands(&self) -> Vec<CommandInfo> {
        self.0
            .session
            .registry
            .all()
            .iter()
            .filter(|c| policy::allows(c.id, c.mutates).is_ok())
            .map(|c| CommandInfo { id: c.id.to_string(), label: c.label.to_string(), menu: c.location.to_string(), mutates: c.mutates })
            .collect()
    }

    fn dirty(&self) -> bool {
        self.0.session.dirty
    }

    fn revision(&self) -> u64 {
        self.0.session.rev()
    }

    fn set_author(&mut self, name: &str) {
        self.0.session.author = name.chars().take(200).collect();
    }

    fn set_policy(&mut self, mut p: Policy) {
        // Hidden tabs stay hidden: Customize Ribbon could show them again.
        if !p.hidden_tabs.is_empty() {
            p.deny.push("tools.customizeRibbon".into());
        }
        policy::set(p);
    }

    fn classify(&self, name: &str) -> Classify {
        // Only the save format counts as the document's save; Save As .odt/.rtf/.pdf are copies.
        let ext = |n: &str| n.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        if ext(name) == ext(&self.save_name()) { Classify::Save } else { Classify::Export }
    }

    fn mark_saved(&mut self) {
        // The app's own save path already cleared `dirty` when the download hook returned Ok.
        self.0.session.dirty = false;
    }
}

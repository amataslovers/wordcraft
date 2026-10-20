//! KawanEditor bridge protocol: the pure state machine between the parent page's SDK and an
//! engine app. No browser APIs here (see `web.rs`), so it is tested natively.
//!
//! Canonical copy: kawan-editor `bridge/src/kawan/protocol.rs`. Engine forks hold a verbatim
//! copy in `apps/<x>craft-web/src/kawan/`; edit the canonical file and run `scripts/sync-bridge.sh`.

use serde_json::{Value, json};

// Engine forks re-export their engine crate's `kawan` module as `policy` (see their `mod.rs`).
use super::policy::{MAX_LOGO_BYTES, Policy, Theme, is_png, parse_hex, window_title};

pub const PROTOCOL: u64 = 1;

const NO_DOCUMENT: &str = "no document is open";

/// Minimum gap between two `edited` events, in milliseconds.
const EDITED_THROTTLE_MS: f64 = 250.0;

pub struct CommandInfo {
    pub id: String,
    pub label: String,
    pub menu: String,
    pub mutates: bool,
}

/// What a file the app saved or downloaded through its own UI is.
pub enum Classify {
    /// The document in its save format: these bytes are the save (the app already produced them).
    Save,
    /// A save of the document in another native format (DeckCraft's web Ctrl+S writes `.deckcraft`):
    /// re-serialize it in the save format.
    #[allow(dead_code)] // only engines with such a format (deck) construct it
    Convert,
    /// A copy in another format (Export as PDF/CSV/PNG, Save As .odt…): handed to the page as is.
    Export,
}

/// A file the app saved or downloaded through its own UI. `export` is set by hooks that only ever
/// see exports (PdfCraft's `download`), so such files never count as a save of the document.
pub struct OutFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub export: bool,
}

/// The engine app, as the bridge drives it.
pub trait Host {
    fn open(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String>;
    /// Serialize the active document in its save format and mark it saved. Returns (file name with extension, bytes).
    fn save(&mut self) -> Result<(String, Vec<u8>), String>;
    /// Same bytes as `save` without marking the document saved (download copy).
    fn export_copy(&mut self) -> Result<(String, Vec<u8>), String>;
    fn exec(&mut self, id: &str, params: Value) -> Result<Value, String>;
    fn commands(&self) -> Vec<CommandInfo>;
    fn dirty(&self) -> bool;
    /// Changes on every document mutation; selection-only changes should not move it.
    fn revision(&self) -> u64;
    fn set_author(&mut self, name: &str);
    /// The host page's policy, from `init` (upstream behaviour when it sent none).
    fn set_policy(&mut self, _policy: Policy) {}
    fn classify(&self, name: &str) -> Classify;
    /// The app produced the save bytes itself (`Classify::Save`): mark the document saved, as
    /// far as the app's own save path has not already.
    fn mark_saved(&mut self);
}

/// One message to post to the parent: `{kawan: 1, type: kind, id?, payload}`, with `bytes`
/// (when present) added as `payload.bytes`.
pub struct Post {
    pub id: Option<u64>,
    pub kind: &'static str,
    pub payload: Value,
    pub bytes: Option<Vec<u8>>,
}

impl Post {
    fn new(id: Option<u64>, kind: &'static str, payload: Value) -> Self {
        Self { id, kind, payload, bytes: None }
    }

    fn error(id: Option<u64>, code: &str, message: impl Into<String>) -> Self {
        Self::new(id, "error", json!({"code": code, "message": message.into()}))
    }

    fn saved(id: Option<u64>, reason: &str, (name, bytes): (String, Vec<u8>)) -> Self {
        let file_type = file_type(&name);
        Self { id, kind: "saved", payload: json!({"name": name, "fileType": file_type, "reason": reason}), bytes: Some(bytes) }
    }
}

fn file_type(name: &str) -> String {
    name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default()
}

#[derive(Default)]
pub struct Protocol {
    initialized: bool,
    /// A document from the page is open; until then the app shows a blank document that must
    /// never be handed to the page as a save.
    opened: bool,
    last_dirty: Option<bool>,
    last_rev: Option<u64>,
    last_edited_ms: Option<f64>,
    edited_pending: bool,
    /// Name of the last successfully opened document.
    name: String,
    title_sent: Option<String>,
}

impl Protocol {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ready(engine: &str, version: &str, upstream_sha: &str) -> Post {
        Post::new(None, "ready", json!({"engine": engine, "version": version, "upstreamSha": upstream_sha, "protocol": PROTOCOL}))
    }

    /// The app could not start (eframe `WebRunner::start` failed): GPU problems get their own code.
    pub fn start_failed(error: &str) -> Post {
        let lower = error.to_ascii_lowercase();
        let gpu = ["webgl", "webgpu", "adapter", "gpu"].iter().any(|w| lower.contains(w));
        Post::error(None, if gpu { "GPU_UNAVAILABLE" } else { "ENGINE_LOAD_FAILED" }, error)
    }

    /// `msg` is the message object without `payload.bytes`; `bytes` is that field when present.
    pub fn handle(&mut self, msg: &Value, bytes: Option<Vec<u8>>, host: &mut dyn Host) -> Vec<Post> {
        let id = msg.get("id").and_then(Value::as_u64);
        if msg.get("kawan").and_then(Value::as_u64) != Some(PROTOCOL) {
            return vec![Post::error(id, "PROTOCOL_MISMATCH", format!("expected kawan protocol {PROTOCOL}"))];
        }
        let kind = msg.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
        let text = |key: &str| payload.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        if kind != "init" && !self.initialized {
            return vec![Post::error(id, "NOT_INITIALIZED", "send init first")];
        }
        let post = match kind {
            "init" => {
                self.initialized = true;
                let mut policy = parse_policy(payload.get("policy"));
                policy.logo_png = bytes.filter(|b| is_png(b) && b.len() <= MAX_LOGO_BYTES).map(Into::into);
                host.set_policy(policy);
                let author = text("author");
                if !author.trim().is_empty() {
                    host.set_author(author.trim());
                }
                Post::new(id, "initialized", json!({}))
            }
            "open" => match bytes {
                None => Post::error(id, "OPEN_FAILED", "missing bytes"),
                Some(bytes) => match host.open(&text("name"), bytes) {
                    Ok(()) => {
                        self.opened = true;
                        self.name = text("name");
                        Post::new(id, "opened", json!({}))
                    }
                    Err(e) => Post::error(id, "OPEN_FAILED", e),
                },
            },
            "save" | "download" if !self.opened => Post::error(id, "SAVE_FAILED", NO_DOCUMENT),
            "exec" if !self.opened => Post::new(id, "execResult", json!({"ok": false, "error": NO_DOCUMENT})),
            "save" => {
                let reason = Some(text("reason")).filter(|r| !r.is_empty()).unwrap_or_else(|| "manual".into());
                match host.save() {
                    Ok(saved) => Post::saved(id, &reason, saved),
                    Err(e) => Post::error(id, "SAVE_FAILED", e),
                }
            }
            "download" => match host.export_copy() {
                Ok(saved) => Post::saved(id, "download", saved),
                Err(e) => Post::error(id, "SAVE_FAILED", e),
            },
            "exec" => {
                let params = payload.get("params").cloned().unwrap_or_else(|| json!({}));
                match host.exec(&text("command"), params) {
                    Ok(result) => Post::new(id, "execResult", json!({"ok": true, "result": result})),
                    Err(e) => Post::new(id, "execResult", json!({"ok": false, "error": e})),
                }
            }
            "listCommands" => {
                let list: Vec<Value> = host
                    .commands()
                    .into_iter()
                    .map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "mutates": c.mutates}))
                    .collect();
                Post::new(id, "commands", json!({"list": list}))
            }
            other => Post::error(id, "UNKNOWN_MESSAGE", format!("unknown message type {other:?}")),
        };
        vec![post]
    }

    /// The window title for the open document, once per change of its name.
    pub fn take_title(&mut self) -> Option<String> {
        if !self.opened || self.title_sent.as_deref() == Some(&self.name) {
            return None;
        }
        self.title_sent = Some(self.name.clone());
        Some(window_title(&self.name))
    }

    /// Once per frame: files the app tried to download leave as `saved` (a save of the document,
    /// re-serialized in its save format) or `exported`; dirty and edited changes are reported.
    pub fn tick(&mut self, now_ms: f64, outbox: Vec<OutFile>, host: &mut dyn Host) -> Vec<Post> {
        let mut posts = Vec::new();
        for file in outbox {
            let kind = if file.export { Classify::Export } else { host.classify(&file.name) };
            posts.push(match kind {
                Classify::Export => Post { id: None, kind: "exported", payload: json!({"name": file.name}), bytes: Some(file.bytes) },
                _ if !self.opened => Post::error(None, "SAVE_FAILED", NO_DOCUMENT),
                Classify::Save => {
                    host.mark_saved();
                    Post::saved(None, "user", (file.name, file.bytes))
                }
                Classify::Convert => match host.save() {
                    Ok(saved) => Post::saved(None, "user", saved),
                    Err(e) => Post::error(None, "SAVE_FAILED", e),
                },
            });
        }
        if !self.initialized {
            return posts;
        }
        let dirty = host.dirty();
        if self.last_dirty != Some(dirty) {
            self.last_dirty = Some(dirty);
            posts.push(Post::new(None, "dirty", json!({"dirty": dirty})));
        }
        let rev = host.revision();
        match self.last_rev {
            None => self.last_rev = Some(rev),
            Some(last) if last != rev => {
                self.last_rev = Some(rev);
                self.edited_pending = true;
            }
            Some(_) => {}
        }
        if self.edited_pending && self.last_edited_ms.is_none_or(|t| now_ms - t >= EDITED_THROTTLE_MS) {
            self.edited_pending = false;
            self.last_edited_ms = Some(now_ms);
            posts.push(Post::new(None, "edited", json!({})));
        }
        posts
    }
}

/// `PolicyJson` (spec §5.1) as the engine's [`Policy`]. Fields of the wrong type fall back to
/// "unrestricted", so a malformed policy never locks the user out.
pub fn parse_policy(v: Option<&Value>) -> Policy {
    let strings = |key: &str| v.and_then(|v| v.get(key)).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
    Policy {
        read_only: v.and_then(|v| v.get("readOnly")).and_then(Value::as_bool).unwrap_or(false),
        allow: strings("allow"),
        deny: strings("deny").unwrap_or_default(),
        hidden_tabs: strings("hiddenTabs").unwrap_or_default(),
        mutating: Vec::new(),
        queries: Vec::new(),
        theme: match v.and_then(|v| v.get("theme")).and_then(Value::as_str) {
            Some("light") => Theme::Light,
            Some("dark") => Theme::Dark,
            _ => Theme::System,
        },
        accent: v.and_then(|v| v.get("accent")).and_then(Value::as_str).and_then(parse_hex),
        logo_png: None,
        logo_href: v
            .and_then(|v| v.get("logo"))
            .and_then(|l| l.get("href"))
            .and_then(Value::as_str)
            .filter(|h| h.strip_prefix("https://").or_else(|| h.strip_prefix("http://")).is_some_and(|rest| !rest.is_empty()))
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[derive(Default)]
    struct FakeHost {
        opened: Option<(String, Vec<u8>)>,
        open_err: Option<String>,
        has_doc: bool,
        marked_saved: bool,
        author: String,
        dirty: bool,
        rev: u64,
        save_calls: u32,
        mark_calls: u32,
        policy: Option<Policy>,
    }

    impl Host for FakeHost {
        fn open(&mut self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
            if let Some(e) = &self.open_err {
                return Err(e.clone());
            }
            self.opened = Some((name.to_string(), bytes));
            self.has_doc = true;
            Ok(())
        }
        fn save(&mut self) -> Result<(String, Vec<u8>), String> {
            self.save_calls += 1;
            let r = self.export_copy()?;
            self.marked_saved = true;
            Ok(r)
        }
        fn mark_saved(&mut self) {
            self.mark_calls += 1;
            self.marked_saved = true;
        }
        fn export_copy(&mut self) -> Result<(String, Vec<u8>), String> {
            if !self.has_doc {
                return Err("no document".into());
            }
            Ok(("a.docx".into(), vec![9, 9]))
        }
        fn exec(&mut self, id: &str, _params: Value) -> Result<Value, String> {
            if id == "bad" { Err("nope".into()) } else { Ok(json!({"done": id})) }
        }
        fn commands(&self) -> Vec<CommandInfo> {
            vec![CommandInfo { id: "file.save".into(), label: "Save".into(), menu: "File".into(), mutates: false }]
        }
        fn dirty(&self) -> bool {
            self.dirty
        }
        fn revision(&self) -> u64 {
            self.rev
        }
        fn set_author(&mut self, name: &str) {
            self.author = name.to_string();
        }
        fn set_policy(&mut self, policy: Policy) {
            self.policy = Some(policy);
        }
        fn classify(&self, name: &str) -> Classify {
            if name.ends_with(".pdf") {
                Classify::Export
            } else if name.ends_with(".deckcraft") {
                Classify::Convert
            } else {
                Classify::Save
            }
        }
    }

    fn msg(kind: &str, id: u64, payload: Value) -> Value {
        json!({"kawan": 1, "type": kind, "id": id, "payload": payload})
    }

    fn ready() -> (Protocol, FakeHost) {
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        p.handle(&msg("init", 1, json!({"policy": {}})), None, &mut h);
        (p, h)
    }

    /// Initialized and a document opened (the normal state for save/exec).
    fn opened() -> (Protocol, FakeHost) {
        let (mut p, mut h) = ready();
        p.handle(&msg("open", 2, json!({"name": "a.docx"})), Some(vec![1]), &mut h);
        (p, h)
    }

    fn out(name: &str, bytes: Vec<u8>, export: bool) -> OutFile {
        OutFile { name: name.into(), bytes, export }
    }

    fn one(posts: Vec<Post>) -> Post {
        assert_eq!(posts.len(), 1, "expected one post");
        posts.into_iter().next().unwrap_or_else(|| unreachable!())
    }

    #[test]
    fn request_before_init_is_not_initialized() {
        let mut p = Protocol::new();
        let post = one(p.handle(&msg("open", 7, json!({"name": "a.docx"})), Some(vec![1]), &mut FakeHost::default()));
        assert_eq!((post.id, post.kind), (Some(7), "error"));
        assert_eq!(post.payload["code"], "NOT_INITIALIZED");
    }

    #[test]
    fn wrong_protocol_version_is_mismatch() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&json!({"kawan": 2, "type": "save", "id": 3}), None, &mut h));
        assert_eq!(post.payload["code"], "PROTOCOL_MISMATCH");
        assert_eq!(post.id, Some(3));
    }

    #[test]
    fn unknown_type_is_unknown_message() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("frobnicate", 4, json!({})), None, &mut h));
        assert_eq!(post.payload["code"], "UNKNOWN_MESSAGE");
    }

    #[test]
    fn init_sets_author_and_replies_initialized() {
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        let post = one(p.handle(&msg("init", 1, json!({"policy": {"readOnly": false}, "author": "Salwa"})), None, &mut h));
        assert_eq!((post.id, post.kind), (Some(1), "initialized"));
        assert_eq!(h.author, "Salwa");
    }

    #[test]
    fn init_hands_the_parsed_policy_to_the_host() {
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        let policy = json!({"readOnly": true, "allow": ["a.*"], "deny": ["b"], "hiddenTabs": ["Review"], "theme": "dark"});
        p.handle(&msg("init", 1, json!({ "policy": policy })), None, &mut h);
        let strings = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let want = Policy { read_only: true, allow: Some(strings(&["a.*"])), deny: strings(&["b"]), hidden_tabs: strings(&["Review"]), theme: Theme::Dark, ..Policy::default() };
        assert_eq!(h.policy, Some(want));
    }

    #[test]
    fn malformed_policy_falls_back_to_upstream() {
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        p.handle(&msg("init", 1, json!({"policy": {"readOnly": "yes", "allow": "x", "deny": [1, "c"], "hiddenTabs": null}})), None, &mut h);
        assert_eq!(h.policy, Some(Policy { deny: vec!["c".to_string()], ..Policy::default() }));
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        p.handle(&msg("init", 1, json!({})), None, &mut h);
        assert_eq!(h.policy, Some(Policy::default()));
    }

    #[test]
    fn open_replies_opened_and_passes_bytes() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("open", 2, json!({"name": "a.docx"})), Some(vec![1, 2, 3]), &mut h));
        assert_eq!((post.id, post.kind), (Some(2), "opened"));
        assert_eq!(h.opened, Some(("a.docx".to_string(), vec![1, 2, 3])));
    }

    #[test]
    fn open_failure_is_open_failed() {
        let (mut p, mut h) = ready();
        h.open_err = Some("not a docx".into());
        let post = one(p.handle(&msg("open", 2, json!({"name": "a.docx"})), Some(vec![1]), &mut h));
        assert_eq!(post.payload["code"], "OPEN_FAILED");
        assert_eq!(post.payload["message"], "not a docx");
    }

    #[test]
    fn open_without_bytes_is_open_failed() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("open", 2, json!({"name": "a.docx"})), None, &mut h));
        assert_eq!(post.payload["code"], "OPEN_FAILED");
    }

    #[test]
    fn save_replies_saved_with_bytes_and_file_type() {
        let (mut p, mut h) = opened();
        let post = one(p.handle(&msg("save", 5, json!({"reason": "manual"})), None, &mut h));
        assert_eq!((post.id, post.kind), (Some(5), "saved"));
        assert_eq!(post.payload["fileType"], "docx");
        assert_eq!(post.payload["reason"], "manual");
        assert_eq!(post.bytes, Some(vec![9, 9]));
        assert!(h.marked_saved);
    }

    #[test]
    fn save_without_document_reports_save_failed() {
        let (mut p, mut h) = opened();
        h.has_doc = false; // the host lost its document after opening
        let post = one(p.handle(&msg("save", 5, json!({"reason": "auto"})), None, &mut h));
        assert_eq!(post.payload["code"], "SAVE_FAILED");
        assert_eq!(post.payload["message"], "no document");
    }

    #[test]
    fn download_uses_export_copy_and_keeps_dirty() {
        let (mut p, mut h) = opened();
        let post = one(p.handle(&msg("download", 6, json!({})), None, &mut h));
        assert_eq!(post.kind, "saved");
        assert_eq!(post.payload["reason"], "download");
        assert!(!h.marked_saved);
    }

    #[test]
    fn exec_error_is_exec_result_not_protocol_error() {
        let (mut p, mut h) = opened();
        let post = one(p.handle(&msg("exec", 8, json!({"command": "bad"})), None, &mut h));
        assert_eq!(post.kind, "execResult");
        assert_eq!(post.payload["ok"], false);
        assert_eq!(post.payload["error"], "nope");
        let post = one(p.handle(&msg("exec", 9, json!({"command": "good", "params": {"x": 1}})), None, &mut h));
        assert_eq!(post.payload["ok"], true);
        assert_eq!(post.payload["result"]["done"], "good");
    }

    #[test]
    fn list_commands_returns_list() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("listCommands", 10, json!({})), None, &mut h));
        assert_eq!(post.kind, "commands");
        assert_eq!(post.payload["list"][0]["id"], "file.save");
        assert_eq!(post.payload["list"][0]["mutates"], false);
    }

    #[test]
    fn tick_turns_saved_download_into_user_saved_event() {
        let (mut p, mut h) = opened();
        let posts = p.tick(0.0, vec![out("Book.xlsx", vec![7, 7, 7], false)], &mut h);
        let saved: Vec<&Post> = posts.iter().filter(|x| x.kind == "saved").collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, None);
        assert_eq!(saved[0].payload["reason"], "user");
        assert_eq!(saved[0].bytes, Some(vec![7, 7, 7]), "the app's own bytes, not a re-serialization");
        assert_eq!((h.save_calls, h.mark_calls), (0, 1));
    }

    #[test]
    fn tick_turns_export_into_exported_event() {
        let (mut p, mut h) = ready();
        let posts = p.tick(0.0, vec![out("Book.pdf", vec![4, 2], false)], &mut h);
        let exported: Vec<&Post> = posts.iter().filter(|x| x.kind == "exported").collect();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].payload["name"], "Book.pdf");
        assert_eq!(exported[0].bytes, Some(vec![4, 2]));
    }

    #[test]
    fn tick_reports_dirty_only_on_change() {
        let (mut p, mut h) = ready();
        let dirty_posts = |posts: Vec<Post>| posts.into_iter().filter(|x| x.kind == "dirty").count();
        assert_eq!(dirty_posts(p.tick(0.0, vec![], &mut h)), 1);
        assert_eq!(dirty_posts(p.tick(1.0, vec![], &mut h)), 0);
        h.dirty = true;
        assert_eq!(dirty_posts(p.tick(2.0, vec![], &mut h)), 1);
        assert_eq!(dirty_posts(p.tick(3.0, vec![], &mut h)), 0);
    }

    #[test]
    fn save_before_open_is_refused() {
        let (mut p, mut h) = ready();
        h.has_doc = true; // the app's blank start document
        let post = one(p.handle(&msg("save", 5, json!({"reason": "manual"})), None, &mut h));
        assert_eq!(post.payload["code"], "SAVE_FAILED");
        assert_eq!(h.save_calls, 0);
    }

    #[test]
    fn download_before_open_is_refused() {
        let (mut p, mut h) = ready();
        h.has_doc = true;
        let post = one(p.handle(&msg("download", 6, json!({})), None, &mut h));
        assert_eq!(post.payload["code"], "SAVE_FAILED");
    }

    #[test]
    fn failed_open_keeps_saves_refused() {
        let (mut p, mut h) = ready();
        h.open_err = Some("bad".into());
        p.handle(&msg("open", 2, json!({"name": "a.docx"})), Some(vec![1]), &mut h);
        h.has_doc = true;
        let post = one(p.handle(&msg("save", 3, json!({})), None, &mut h));
        assert_eq!(post.payload["code"], "SAVE_FAILED");
    }

    #[test]
    fn exec_before_open_is_refused() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("exec", 8, json!({"command": "good"})), None, &mut h));
        assert_eq!(post.payload["ok"], false);
    }

    #[test]
    fn list_commands_works_before_open() {
        let (mut p, mut h) = ready();
        let post = one(p.handle(&msg("listCommands", 10, json!({})), None, &mut h));
        assert_eq!(post.kind, "commands");
    }

    #[test]
    fn tick_user_save_before_open_is_refused() {
        let (mut p, mut h) = ready();
        h.has_doc = true;
        let posts = p.tick(0.0, vec![out("Document1.docx", vec![1], false)], &mut h);
        assert!(posts.iter().any(|x| x.kind == "error" && x.payload["code"] == "SAVE_FAILED"));
        assert!(!posts.iter().any(|x| x.kind == "saved"));
    }

    #[test]
    fn tick_convert_reserializes() {
        let (mut p, mut h) = opened();
        let posts = p.tick(0.0, vec![out("Deck.deckcraft", vec![5], false)], &mut h);
        let saved: Vec<&Post> = posts.iter().filter(|x| x.kind == "saved").collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].bytes, Some(vec![9, 9]), "re-serialized by host.save()");
        assert_eq!(h.save_calls, 1);
    }

    #[test]
    fn tick_export_flag_bypasses_classify() {
        let (mut p, mut h) = opened();
        let posts = p.tick(0.0, vec![out("Split part 1.docx", vec![3], true)], &mut h);
        assert!(posts.iter().any(|x| x.kind == "exported" && x.payload["name"] == "Split part 1.docx"));
        assert_eq!((h.save_calls, h.mark_calls), (0, 0));
    }

    #[test]
    fn start_failed_maps_gpu_errors() {
        assert_eq!(Protocol::start_failed("WebGL2 is not available").payload["code"], "GPU_UNAVAILABLE");
        assert_eq!(Protocol::start_failed("no suitable adapter found").payload["code"], "GPU_UNAVAILABLE");
        assert_eq!(Protocol::start_failed("JsValue(TypeError: x is undefined)").payload["code"], "ENGINE_LOAD_FAILED");
        assert_eq!(Protocol::start_failed("boom").kind, "error");
    }

    #[test]
    fn tick_throttles_edited_to_250ms() {
        let (mut p, mut h) = ready();
        let edited = |posts: Vec<Post>| posts.into_iter().filter(|x| x.kind == "edited").count();
        p.tick(0.0, vec![], &mut h); // baseline revision
        h.rev = 1;
        assert_eq!(edited(p.tick(10.0, vec![], &mut h)), 1);
        h.rev = 2;
        assert_eq!(edited(p.tick(100.0, vec![], &mut h)), 0);
        h.rev = 3;
        assert_eq!(edited(p.tick(200.0, vec![], &mut h)), 0);
        assert_eq!(edited(p.tick(260.0, vec![], &mut h)), 1, "held edit flushes after 250 ms");
        assert_eq!(edited(p.tick(600.0, vec![], &mut h)), 0, "no change, no event");
    }

    const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn init_with(policy: Value, bytes: Option<Vec<u8>>) -> Policy {
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        p.handle(&msg("init", 1, json!({ "policy": policy })), bytes, &mut h);
        h.policy.unwrap_or_default()
    }

    #[test]
    fn init_parses_theme_accent_and_logo() {
        let mut bytes = PNG_SIG.to_vec();
        bytes.extend([0u8; 8]);
        let policy = init_with(json!({"theme": "dark", "accent": "#E11D48", "logo": {"href": "https://example.com/"}}), Some(bytes));
        assert_eq!(policy.theme, Theme::Dark);
        assert_eq!(policy.accent, Some([0xE1, 0x1D, 0x48]));
        assert_eq!(policy.logo_href.as_deref(), Some("https://example.com/"));
        assert_eq!(policy.logo_png.map(|l| l.len()), Some(16));
    }

    #[test]
    fn init_ignores_non_png_or_oversized_logo() {
        assert!(init_with(json!({}), Some(b"<html>".to_vec())).logo_png.is_none());
        let mut big = PNG_SIG.to_vec();
        big.extend(vec![0u8; MAX_LOGO_BYTES]);
        assert!(init_with(json!({}), Some(big)).logo_png.is_none());
        let mut exact = PNG_SIG.to_vec();
        exact.extend(vec![0u8; MAX_LOGO_BYTES - PNG_SIG.len()]);
        assert_eq!(init_with(json!({}), Some(exact)).logo_png.map(|l| l.len()), Some(MAX_LOGO_BYTES));
    }

    #[test]
    fn init_keeps_only_http_logo_href() {
        for bad in [json!("javascript:alert(1)"), json!("/rel"), json!(5), json!("http://"), json!("https://")] {
            assert_eq!(init_with(json!({"logo": {"href": bad}}), None).logo_href, None);
        }
        assert_eq!(init_with(json!({"logo": {"href": "http://a"}}), None).logo_href.as_deref(), Some("http://a"));
    }

    #[test]
    fn malformed_theme_and_accent_fall_back() {
        let policy = init_with(json!({"theme": "blue", "accent": "red"}), None);
        assert_eq!((policy.theme, policy.accent), (Theme::System, None));
        let mut p = Protocol::new();
        let mut h = FakeHost::default();
        p.handle(&msg("init", 1, json!({})), None, &mut h);
        assert_eq!(h.policy, Some(Policy::default()));
    }

    #[test]
    fn open_sets_the_title_once() {
        let (mut p, mut h) = ready();
        assert_eq!(p.take_title(), None);
        h.open_err = Some("bad".into());
        p.handle(&msg("open", 2, json!({"name": "Bad.docx"})), Some(vec![1]), &mut h);
        assert_eq!(p.take_title(), None);
        h.open_err = None;
        p.handle(&msg("open", 3, json!({"name": "Laporan.docx"})), Some(vec![1]), &mut h);
        assert_eq!(p.take_title().as_deref(), Some("Laporan.docx — KawanEditor"));
        assert_eq!(p.take_title(), None);
    }
}

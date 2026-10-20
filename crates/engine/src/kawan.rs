//! KawanEditor command policy: which commands the host allows, read-only mode and hidden ribbon
//! tabs. Pure std so every engine crate can carry it. Canonical copy lives in kawan-editor
//! `bridge/src/kawan/policy.rs`; `scripts/sync-bridge.sh` copies it to each engine as
//! `crates/engine/src/kawan.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PRODUCT: &str = "KawanEditor";
/// Set at build time by `scripts/build-engines.sh` (`KAWAN_VERSION`).
pub const VERSION: &str = match option_env!("KAWAN_VERSION") {
    Some(v) => v,
    None => "dev",
};
/// Largest logo PNG the host may send, in bytes.
pub const MAX_LOGO_BYTES: usize = 262_144;

/// Colour scheme the host asks for; `System` is the engine's own System mode: follow the browser/OS
/// colour scheme (deck reads it once at init).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// What the host page allows. `Default` is upstream behaviour: nothing restricted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Policy {
    pub read_only: bool,
    /// `None` allows every command; `Some` only the matching ones.
    pub allow: Option<Vec<String>>,
    pub deny: Vec<String>,
    pub hidden_tabs: Vec<String>,
    /// Commands that change the document although the engine's own flag says otherwise (undo,
    /// redo, or replacing it with another one: open, new, close); filled in by each engine's
    /// bridge host.
    pub mutating: Vec<String>,
    /// The engine UI's own state queries (`format.state`, `cell.get`, ...) and selection moves:
    /// the host cannot know their ids, so its allow and deny lists skip them (read-only still
    /// applies). Filled in by each engine's bridge host.
    pub queries: Vec<String>,
    pub theme: Theme,
    /// Brand accent colour (sRGB).
    pub accent: Option<[u8; 3]>,
    /// The host's logo, a PNG already checked by the protocol.
    pub logo_png: Option<Rc<[u8]>>,
    /// Where a click on the logo goes (`http` or `https` only).
    pub logo_href: Option<String>,
}

const NOT_ALLOWED: &str = "not allowed by the host";

impl Policy {
    pub fn allows(&self, id: &str, mutates: bool) -> Result<(), &'static str> {
        if self.read_only && (mutates || matches_any(&self.mutating, id)) {
            return Err("read-only");
        }
        if matches_any(&self.queries, id) {
            return Ok(());
        }
        if matches_any(&self.deny, id) {
            return Err(NOT_ALLOWED);
        }
        match &self.allow {
            Some(allow) if !matches_any(allow, id) => Err(NOT_ALLOWED),
            _ => Ok(()),
        }
    }

    pub fn tab_hidden(&self, tab: &str) -> bool {
        self.hidden_tabs.iter().any(|t| t == tab)
    }
}

fn matches_any(patterns: &[String], id: &str) -> bool {
    patterns.iter().any(|p| glob(p, id))
}

/// `*` matches any run of characters (dots included); everything else matches itself, case-sensitive.
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti) = (0, 0);
    // Position after the last `*` in the pattern, and where in the text it started matching.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if p.get(pi) == Some(&b'*') {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if p.get(pi) == t.get(ti) {
            pi += 1;
            ti += 1;
        } else if let Some((sp, st)) = star {
            pi = sp;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi.min(p.len())..].iter().all(|&c| c == b'*')
}

/// The "about" line shown by the engine; `upstream` is the engine's product name.
pub fn about(upstream: &str) -> String {
    format!("{PRODUCT} {VERSION} — based on {upstream} by the ArtCraft team. Licensed MIT OR Apache-2.0.")
}

/// Browser tab title for the open document.
pub fn window_title(doc: &str) -> String {
    let doc = doc.trim();
    if doc.is_empty() { PRODUCT.to_string() } else { format!("{doc} — {PRODUCT}") }
}

/// `#RRGGBB`, case-insensitive; anything else is `None`.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

pub fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

/// Linear blend from `a` (t = 0) to `b` (t = 1); `t` is clamped.
pub fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let ch = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t).round() as u8;
    [ch(0), ch(1), ch(2)]
}

fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG 2.x contrast ratio, 1.0 to 21.0.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// White or near-black, whichever reads better on `bg`.
pub fn on_color(bg: [u8; 3]) -> [u8; 3] {
    const LIGHT: [u8; 3] = [255, 255, 255];
    const DARK: [u8; 3] = [17, 17, 17];
    if contrast(bg, LIGHT) >= contrast(bg, DARK) { LIGHT } else { DARK }
}

thread_local! {
    static POLICY: RefCell<Policy> = RefCell::new(Policy::default());
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static HOST_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Install the host's policy (once, on `init`).
pub fn set(policy: Policy) {
    POLICY.with(|p| *p.borrow_mut() = policy);
    GENERATION.with(|g| g.set(g.get() + 1));
}

/// Moves on every [`set`]; engines key texture caches and theme re-application on it.
pub fn generation() -> u64 {
    GENERATION.with(Cell::get)
}

pub fn theme() -> Theme {
    POLICY.with(|p| p.borrow().theme)
}

pub fn accent() -> Option<[u8; 3]> {
    POLICY.with(|p| p.borrow().accent)
}

pub fn logo_png() -> Option<Rc<[u8]>> {
    POLICY.with(|p| p.borrow().logo_png.clone())
}

pub fn logo_href() -> Option<String> {
    POLICY.with(|p| p.borrow().logo_href.clone())
}

pub fn allows(id: &str, mutates: bool) -> Result<(), &'static str> {
    if HOST_DEPTH.with(Cell::get) > 0 {
        return Ok(());
    }
    POLICY.with(|p| p.borrow().allows(id, mutates))
}

pub fn tab_hidden(tab: &str) -> bool {
    POLICY.with(|p| p.borrow().tab_hidden(tab))
}

pub fn read_only() -> bool {
    POLICY.with(|p| p.borrow().read_only)
}

/// Run `f` as the bridge itself (open, save, download): no command check refuses it. Read-only
/// still holds where engines check it directly (their edit chokepoints call [`read_only`]).
pub fn as_host<R>(f: impl FnOnce() -> R) -> R {
    HOST_DEPTH.with(|d| d.set(d.get() + 1));
    let r = f();
    HOST_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn glob_matches_exact_and_stars() {
        assert!(glob("file.save", "file.save"));
        assert!(!glob("file.save", "file.saveAs"));
        assert!(glob("review.*", "review.comment.new"));
        assert!(glob("file.export*", "file.exportPdf"));
        assert!(glob("*.delete", "slide.delete"));
        assert!(glob("a*b*c", "aXbYc"));
        assert!(!glob("a*b*c", "aXc"));
        assert!(glob("*", ""));
        assert!(glob("a**", "a"));
        assert!(!glob("File.save", "file.save"));
        assert!(!glob("", "x"));
    }

    #[test]
    fn deny_beats_allow() {
        let p = Policy { allow: Some(strings(&["file.*"])), deny: strings(&["file.saveAs"]), ..Policy::default() };
        assert_eq!(p.allows("file.saveAs", false), Err(NOT_ALLOWED));
        assert_eq!(p.allows("file.save", false), Ok(()));
        assert_eq!(p.allows("edit.copy", false), Err(NOT_ALLOWED));
    }

    #[test]
    fn allow_none_vs_empty() {
        assert_eq!(Policy::default().allows("anything", false), Ok(()));
        let none = Policy { allow: Some(vec![]), ..Policy::default() };
        assert_eq!(none.allows("anything", false), Err(NOT_ALLOWED));
    }

    #[test]
    fn read_only_refuses_mutations_and_listed_ids() {
        let p = Policy { read_only: true, mutating: strings(&["edit.undo"]), ..Policy::default() };
        assert_eq!(p.allows("text.insert", true), Err("read-only"));
        assert_eq!(p.allows("edit.undo", false), Err("read-only"));
        assert_eq!(p.allows("view.zoom", false), Ok(()));
        // `mutating` only matters when read-only.
        let rw = Policy { mutating: strings(&["edit.undo"]), ..Policy::default() };
        assert_eq!(rw.allows("edit.undo", false), Ok(()));
    }

    #[test]
    fn queries_skip_the_host_lists_but_not_read_only() {
        let p = Policy {
            read_only: true,
            allow: Some(vec![]),
            deny: strings(&["format.*"]),
            queries: strings(&["format.state", "list.get"]),
            mutating: strings(&["list.get"]),
            ..Policy::default()
        };
        assert_eq!(p.allows("format.state", false), Ok(()));
        assert_eq!(p.allows("format.bold", false), Err(NOT_ALLOWED));
        assert_eq!(p.allows("list.get", false), Err("read-only"));
    }

    #[test]
    fn tab_hidden_is_exact() {
        let p = Policy { hidden_tabs: strings(&["Review"]), ..Policy::default() };
        assert!(p.tab_hidden("Review"));
        assert!(!p.tab_hidden("review"));
        assert!(!p.tab_hidden("Home"));
    }

    #[test]
    fn global_defaults_to_upstream_and_set_replaces() {
        assert_eq!(allows("anything", true), Ok(()));
        assert!(!read_only());
        set(Policy { read_only: true, hidden_tabs: strings(&["Review"]), ..Policy::default() });
        assert!(read_only());
        assert!(tab_hidden("Review"));
        set(Policy::default());
        assert!(!read_only());
        assert!(!tab_hidden("Review"));
    }

    #[test]
    fn as_host_suspends_the_command_checks_but_not_read_only() {
        set(Policy { read_only: true, deny: strings(&["file.*"]), mutating: strings(&["file.open"]), ..Policy::default() });
        // The bridge's own open/save run as the host: no command check refuses them...
        assert_eq!(as_host(|| allows("file.save", false)), Ok(()));
        assert_eq!(as_host(|| as_host(|| allows("file.open", false))), Ok(()));
        // ...while the engines' edit chokepoints still see read-only.
        assert!(as_host(read_only));
        assert_eq!(allows("file.save", false), Err(NOT_ALLOWED));
        assert_eq!(allows("file.open", false), Err("read-only"));
        // A Policy value is pure: only the global check knows about the bridge.
        let p = Policy { deny: strings(&["file.*"]), ..Policy::default() };
        assert_eq!(as_host(|| p.allows("file.save", false)), Err(NOT_ALLOWED));
        set(Policy::default());
    }

    #[test]
    fn about_and_title_texts() {
        assert_eq!(about("PdfCraft"), format!("KawanEditor {VERSION} — based on PdfCraft by the ArtCraft team. Licensed MIT OR Apache-2.0."));
        assert_eq!(window_title("a.docx"), "a.docx — KawanEditor");
        assert_eq!(window_title("  "), "KawanEditor");
    }

    #[test]
    fn parse_hex_accepts_only_rrggbb() {
        assert_eq!(parse_hex("#1e5eff"), Some([0x1E, 0x5E, 0xFF]));
        assert_eq!(parse_hex("#1E5EFF"), Some([0x1E, 0x5E, 0xFF]));
        for bad in ["1E5EFF", "#1E5EF", "#GGGGGG", "", "#1E5EFF0"] {
            assert_eq!(parse_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn mix_and_contrast() {
        assert_eq!(mix([0, 0, 0], [255, 255, 255], 0.5), [128, 128, 128]);
        assert_eq!(mix([1, 2, 3], [9, 9, 9], -1.0), [1, 2, 3]);
        assert_eq!(mix([1, 2, 3], [9, 9, 9], 2.0), [9, 9, 9]);
        assert!((contrast([255, 255, 255], [0, 0, 0]) - 21.0).abs() < 0.01);
        assert_eq!(contrast([40, 90, 200], [40, 90, 200]), 1.0);
    }

    #[test]
    fn on_color_picks_the_more_readable() {
        assert_eq!(on_color([255, 255, 255]), [17, 17, 17]);
        assert_eq!(on_color([0, 0, 0]), [255, 255, 255]);
        assert_eq!(on_color([255, 255, 0]), [17, 17, 17]);
        assert_eq!(on_color([0x1E, 0x5E, 0xFF]), [255, 255, 255]);
    }

    #[test]
    fn is_png_checks_the_signature() {
        assert!(is_png(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1]));
        assert!(!is_png(b"<html>"));
        assert!(!is_png(&[]));
    }

    #[test]
    fn generation_moves_on_set() {
        let g = generation();
        set(Policy::default());
        assert!(generation() > g);
    }

    #[test]
    fn branding_accessors_follow_set() {
        set(Policy { theme: Theme::Dark, accent: Some([1, 2, 3]), logo_href: Some("https://x".into()), logo_png: Some(vec![1u8].into()), ..Policy::default() });
        assert_eq!(theme(), Theme::Dark);
        assert_eq!(accent(), Some([1, 2, 3]));
        assert_eq!(logo_href().as_deref(), Some("https://x"));
        assert_eq!(logo_png().as_deref(), Some(&[1u8][..]));
        set(Policy::default());
        assert_eq!((theme(), accent(), logo_href(), logo_png()), (Theme::System, None, None, None));
    }
}

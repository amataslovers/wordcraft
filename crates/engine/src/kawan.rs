//! KawanEditor command policy: which commands the host allows, read-only mode and hidden ribbon
//! tabs. Pure std so every engine crate can carry it. Canonical copy lives in kawan-editor
//! `bridge/src/kawan/policy.rs`; `scripts/sync-bridge.sh` copies it to each engine as
//! `crates/engine/src/kawan.rs`.

use std::cell::{Cell, RefCell};

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

thread_local! {
    static POLICY: RefCell<Policy> = RefCell::new(Policy::default());
    static HOST_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Install the host's policy (once, on `init`).
pub fn set(policy: Policy) {
    POLICY.with(|p| *p.borrow_mut() = policy);
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
}

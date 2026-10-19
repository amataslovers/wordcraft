//! KawanEditor policy (`kawan.rs`): deny and read-only reach `Session::run`, queries keep working
//! in read-only, and the default policy leaves WordCraft as upstream.

use serde_json::json;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::kawan::{self, Policy};
use wordcraft_engine::{CmdError, Session};

fn session() -> Session {
    Session::new(Document::new())
}

fn text(s: &Session) -> String {
    s.doc.plain_text(StoryRef::Body)
}

#[test]
fn deny_refuses_run() {
    let mut s = session();
    kawan::set(Policy { deny: vec!["text.insert".into()], ..Policy::default() });
    assert!(matches!(s.run("text.insert", &json!({"text": "x"})), Err(CmdError::Disabled(_))));
    assert_eq!(text(&s), "");
    kawan::set(Policy::default());
}

#[test]
fn read_only_refuses_mutations() {
    let mut s = session();
    kawan::set(Policy { read_only: true, ..Policy::default() });
    assert!(matches!(s.run("text.insert", &json!({"text": "x"})), Err(CmdError::Disabled(_))));
    assert!(matches!(s.run("para.alignCenter", &json!({})), Err(CmdError::Disabled(_))));
    assert_eq!(text(&s), "");
    kawan::set(Policy::default());
}

#[test]
fn read_only_allows_queries() {
    let mut s = session();
    kawan::set(Policy { read_only: true, ..Policy::default() });
    // The ribbon reads these every frame.
    assert!(s.run("format.state", &json!({})).is_ok());
    assert!(s.run("file.info", &json!({})).is_ok());
    kawan::set(Policy::default());
}

#[test]
fn upstream_policy_changes_nothing() {
    let mut s = session();
    kawan::set(Policy::default());
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    assert_eq!(text(&s), "x");
}

#[test]
fn read_only_refuses_undo_listed_as_mutating() {
    let mut s = session();
    s.run("text.insert", &json!({"text": "x"})).unwrap();
    // `edit.undo` is pure in the registry; the bridge host lists it as mutating.
    kawan::set(Policy { read_only: true, mutating: vec!["edit.undo".into(), "edit.repeat".into()], ..Policy::default() });
    assert!(matches!(s.run("edit.undo", &json!({})), Err(CmdError::Disabled(_))));
    assert!(matches!(s.run("edit.repeat", &json!({})), Err(CmdError::Disabled(_))));
    assert_eq!(text(&s), "x");
    kawan::set(Policy::default());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s), "");
}

#[test]
fn allow_and_deny_spare_the_queries_the_ribbon_reads() {
    let mut s = session();
    // The host knows nothing of `format.state`; the ribbon reads it every frame. The bridge host
    // lists such queries in `Policy::queries`.
    let queries = vec!["format.state".to_string(), "file.info".to_string()];
    kawan::set(Policy { allow: Some(vec!["text.insert".into()]), queries: queries.clone(), ..Policy::default() });
    assert!(s.run("format.state", &json!({})).is_ok());
    assert!(s.run("file.info", &json!({})).is_ok());
    assert!(matches!(s.run("format.bold", &json!({})), Err(CmdError::Disabled(_))));
    kawan::set(Policy { deny: vec!["format.*".into()], queries, ..Policy::default() });
    assert!(s.run("format.state", &json!({})).is_ok());
    assert!(matches!(s.run("format.bold", &json!({})), Err(CmdError::Disabled(_))));
    kawan::set(Policy::default());
}

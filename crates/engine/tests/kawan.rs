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

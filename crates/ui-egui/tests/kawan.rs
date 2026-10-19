//! KawanEditor policy in the WordCraft UI: hidden tabs are not drawn, a hidden active tab falls
//! back to a visible one, and denied buttons disappear from the ribbon.

use egui_kittest::kittest::Queryable as _;
use wordcraft_engine::Session;
use wordcraft_engine::kawan::{self, Policy};
use wordcraft_ui_egui::{Services, WordApp};

fn harness() -> egui_kittest::Harness<'static, WordApp> {
    let app = WordApp::new(Session::new(wordcraft_doc::Document::new()), Services::default());
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).build_ui_state(
        |ui, app: &mut WordApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

#[test]
fn hidden_tab_is_not_drawn() {
    kawan::set(Policy { hidden_tabs: vec!["Review".into()], ..Policy::default() });
    let h = harness();
    assert!(h.query_by_label("Review").is_none());
    assert!(h.query_by_label("Insert").is_some());
    kawan::set(Policy::default());
}

#[test]
fn hidden_active_tab_falls_back() {
    kawan::set(Policy { hidden_tabs: vec!["Home".into()], ..Policy::default() });
    let mut h = harness();
    h.state_mut().ui.tab = "Home".into();
    h.run_steps(2);
    assert_ne!(h.state().ui.tab, "Home");
    kawan::set(Policy::default());
}

#[test]
fn denied_button_is_not_drawn() {
    kawan::set(Policy::default());
    assert!(harness().query_by_label("Bold").is_some(), "the Home tab shows Bold");
    kawan::set(Policy { deny: vec!["format.bold".into()], ..Policy::default() });
    assert!(harness().query_by_label("Bold").is_none());
    kawan::set(Policy::default());
}

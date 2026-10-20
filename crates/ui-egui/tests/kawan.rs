//! KawanEditor policy in the WordCraft UI: hidden tabs are not drawn, a hidden active tab falls
//! back to a visible one, and denied buttons disappear from the ribbon. Branding: no ArtCraft
//! mark on screen, the KawanEditor logo in the title bar (the host's own when it sends one, a
//! link when it gives an address).

use std::rc::Rc;

use egui_kittest::kittest::Queryable as _;
use serde_json::json;
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
    assert!(h.query_all_by_label("Review").next().is_none());
    assert!(h.query_all_by_label("Insert").next().is_some());
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
    assert!(harness().query_all_by_label("Bold").next().is_some(), "the Home tab shows Bold");
    kawan::set(Policy { deny: vec!["format.bold".into()], ..Policy::default() });
    assert!(harness().query_all_by_label("Bold").next().is_none());
    kawan::set(Policy::default());
}

/// No ArtCraft mark and no community link; the attribution sentence of the About line is allowed.
/// A `Role::Label` node (`ui.label`) keeps its text in `value`, every other node in `label`.
fn no_artcraft(h: &egui_kittest::Harness<'static, WordApp>, place: &str) {
    let mark = h
        .query_all_by(|n| {
            [n.label(), n.value()]
                .into_iter()
                .flatten()
                .any(|l| (l.to_ascii_lowercase().contains("artcraft") && !l.contains("by the ArtCraft team")) || l.contains("Discord"))
        })
        .next();
    assert!(mark.is_none(), "{place}: {mark:?}");
}

/// The topmost node labelled `label` (`get_all`: About and the title bar can both carry one).
fn topmost<'a>(h: &'a egui_kittest::Harness<'static, WordApp>, label: &'a str) -> egui_kittest::Node<'a> {
    h.get_all_by_label(label).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).expect("get_all_by_label found one")
}

/// A 4 x 4 opaque red PNG.
fn red_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

#[test]
fn no_artcraft_label_in_home_and_about() {
    kawan::set(Policy::default());
    let mut h = harness();
    h.state_mut().start_screen();
    h.run_steps(4);
    no_artcraft(&h, "File > Home");
    assert!(h.query_all_by_label("About").next().is_some(), "the Backstage keeps its About entry");
    h.state_mut().run("ui.backstage", json!({"value": false})).unwrap();
    h.state_mut().ui.tab = "Help".into();
    h.run_steps(4);
    no_artcraft(&h, "Help tab");
    h.state_mut().run("ui.dialog", json!({"name": "about"})).unwrap();
    h.run_steps(4);
    no_artcraft(&h, "About");
    assert!(h.query_all_by_label("About KawanEditor").next().is_some(), "the dialog title");
    let about = kawan::about("WordCraft");
    assert!(h.query_all_by_label(&about).next().is_some(), "the About line {about:?}");
    assert!(h.query_all_by_label("KawanEditor").count() >= 2, "the About logo and heading (and the title bar logo)");
    assert!(h.query_all_by_label("Source code: github.com/storytold/wordcraft").next().is_some(), "the upstream source link stays");
    // The command stays registered but opens nothing.
    h.state_mut().run("ui.discord", json!({})).unwrap();
    assert_eq!(h.state().canvas.open_url, None);
}

#[test]
fn header_logo_is_drawn() {
    kawan::set(Policy::default());
    let h = harness();
    let logo = topmost(&h, "KawanEditor").rect();
    let tab = topmost(&h, "Insert").rect();
    assert!(logo.bottom() <= tab.top(), "in the title bar, above the ribbon tabs: {logo:?} {tab:?}");
}

#[test]
fn host_logo_with_href_opens_it() {
    kawan::set(Policy { logo_png: Some(Rc::from(red_png())), logo_href: Some("https://example.com/".into()), ..Policy::default() });
    let mut h = harness();
    topmost(&h, "KawanEditor").click();
    let mut opened = None;
    for _ in 0..4 {
        h.step();
        opened = opened.or_else(|| {
            h.output().platform_output.commands.iter().find_map(|c| match c {
                egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
                _ => None,
            })
        });
    }
    assert_eq!(opened.as_deref(), Some("https://example.com/"));
    let logo = wordcraft_ui_egui::kawan_brand::logo_texture(&h.ctx).map(|t| t.size());
    assert_eq!(logo, Some([4, 4]), "the host's PNG, not the default");
    kawan::set(Policy::default());
}

#[test]
fn host_logo_without_href_is_not_a_link() {
    kawan::set(Policy { logo_png: Some(Rc::from(b"not a png".to_vec())), ..Policy::default() });
    let mut h = harness();
    topmost(&h, "KawanEditor").click();
    for _ in 0..4 {
        h.step();
        assert!(!h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::OpenUrl(_))));
    }
    let logo = wordcraft_ui_egui::kawan_brand::logo_texture(&h.ctx).map(|t| t.size());
    assert_eq!(logo, Some([256, 256]), "a bad host PNG falls back to the default logo");
    kawan::set(Policy::default());
}

//! Offscreen visual QA for the dock's priority panels — Layers, Properties, Adjustments, Color,
//! History and Brushes — in a dark theme, four light themes and at 125% UI scale.
//!
//! The assertions run with the normal `--lib` suite: each body stays inside its 340×640 dock
//! column without horizontal overflow, the tab strip and the sticky footers stay inside the
//! group, and the empty states and primary actions are fully visible. The ignored writer renders
//! the same states through wgpu to PNGs for a human look:
//! `cargo test -p photocraft-ui-egui --lib dock_visual_qa -- --ignored`.

use egui::{Rect, Vec2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::PhotocraftApp;
use crate::dock::Group;
use crate::theme::{ThemeKind, Tokens};

/// One dark theme and the four light themes the dock must read in.
const THEMES: [ThemeKind; 5] = [ThemeKind::Studio, ThemeKind::StudioLight, ThemeKind::SoftLight, ThemeKind::ProLight, ThemeKind::ProLightGray];

/// Where the screenshot writer puts its PNGs.
fn output_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/private/var/folders/30/m6f55_095bs3mldd6bmn06tm0000gn/T/opencode/ui-dock")
}

fn app(empty: bool, group: Group, tab: usize, text: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    if !empty {
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        if text {
            app.run("type.create", json!({"x": 8, "y": 50, "text": "PhotoCraft", "size": 24})).expect("a type layer");
        } else {
            app.run("layer.new.layer", json!({})).expect("a second layer");
        }
        match (group, tab) {
            (Group::History, 1) => {
                app.run("actions.record", json!({"name": "Add a layer"})).expect("record an action");
                app.run("layer.new.layer", json!({})).expect("a recorded step");
                app.run("actions.stop", json!({})).expect("stop recording");
            }
            (Group::History, 2) => {
                app.run("layerComp.new", json!({"name": "Layout A"})).expect("a layer comp");
            }
            (Group::Layers, 2) => {
                app.run("path.set", json!({"name": "Path 1", "path": {"subpaths": [{"closed": true, "knots": [[8.0, 8.0], [56.0, 8.0], [32.0, 56.0]]}]}}))
                    .expect("a saved path");
            }
            _ => {}
        }
        app.sync_views();
    }
    // One group only: it fills the column, so the body height is the dock height.
    for g in Group::ALL {
        *g.shown_mut(&mut app.ui.panels) = g == group;
    }
    app.ui.panels.rail = false;
    app.ui.dock = crate::dock::DockLayout::default();
    *group.tab_mut(&mut app.ui.dock_tabs) = tab;
    app
}

/// The real right dock inside a 1440×900 window, with only `group` shown.
fn draw_dock(app: PhotocraftApp, theme: ThemeKind, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.ctx().request_repaint();
                return;
            }
            crate::panels::right_dock(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if scale != 1.0 {
        h.ctx.set_zoom_factor(scale);
    }
    h.run_steps(5);
    h
}

/// A single dock body in a `width`×640 column, with its body and content bounds recorded.
fn draw_body(app: PhotocraftApp, theme: ThemeKind, scale: f32, width: f32, group: Group, tab: usize) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(width, 640.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.ctx().request_repaint();
                return;
            }
            ui.painter().rect_filled(ui.max_rect(), 0.0, Tokens::get(ui.ctx()).card);
            let body = ui.max_rect();
            let content = ui.scope(|ui| crate::panels::dock_body(app, ui, group, tab)).response.rect;
            ui.ctx().data_mut(|d| d.insert_temp(metrics_id(group, tab), (body, content)));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if scale != 1.0 {
        h.ctx.set_zoom_factor(scale);
    }
    h.run_steps(5);
    h
}

fn metrics_id(group: Group, tab: usize) -> egui::Id {
    egui::Id::new(("dock-qa-body", group.key(), tab))
}

fn body_metrics(h: &Harness<'_, PhotocraftApp>, group: Group, tab: usize) -> (Rect, Vec2) {
    let (body, content) = h.ctx.data(|d| d.get_temp::<(Rect, Rect)>(metrics_id(group, tab))).unwrap_or_else(|| panic!("{group:?} tab {tab} recorded its body"));
    (body, content.size())
}

fn inside(outer: Rect, inner: Rect) -> bool {
    inner.left() >= outer.left() - 1.0 && inner.right() <= outer.right() + 1.0 && inner.top() >= outer.top() - 1.0 && inner.bottom() <= outer.bottom() + 1.0
}

/// The smallest visible node whose accessible label starts with `key` (button tooltips carry the
/// shortcut in parentheses, so an exact match is not enough).
fn node_starts(h: &Harness<'_, PhotocraftApp>, key: &str) -> Rect {
    key_rect(h, key).unwrap_or_else(|| panic!("{key} is present"))
}

fn key_rect(h: &Harness<'_, PhotocraftApp>, key: &str) -> Option<Rect> {
    h.query_all_by_label_contains(key).map(|node| node.rect()).min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
}

/// Panels whose content arrives from a worker (the Histogram) need a few frames to first paint;
/// pump the harness briefly instead of failing on the first empty frame.
fn settle(h: &mut Harness<'_, PhotocraftApp>, key: &str) -> Option<Rect> {
    if let Some(rect) = key_rect(h, key) {
        return Some(rect);
    }
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.run_steps(1);
        if let Some(rect) = key_rect(h, key) {
            return Some(rect);
        }
    }
    None
}

/// One priority panel to draw, its key control and the empty-state label it shows without a
/// document.
struct PanelCase {
    name: &'static str,
    group: Group,
    tab: usize,
    key: &'static str,
    empty_key: Option<&'static str>,
    /// The document carries a type layer, so Character and Paragraph show their sections.
    text: bool,
    /// The key sits in the footer bar, which runs over the body's bottom margin in a real dock:
    /// allow that margin before calling it clipped.
    footer: bool,
}

fn cases(pro: bool) -> Vec<PanelCase> {
    // Photoshop's Properties leads with Transform; the other themes' with the layer's Opacity
    // card. Either is the first control a reader meets under the layer header.
    let properties = if pro { "Transform" } else { "Opacity" };
    vec![
        PanelCase { name: "layers", group: Group::Layers, tab: 0, key: "Opacity", empty_key: Some("No document"), text: false, footer: false },
        PanelCase { name: "properties", group: Group::Properties, tab: 0, key: properties, empty_key: Some("No properties"), text: false, footer: false },
        PanelCase { name: "adjustments", group: Group::Properties, tab: 1, key: "Brightness/Contrast", empty_key: None, text: false, footer: false },
        PanelCase { name: "history", group: Group::History, tab: 0, key: "New Layer", empty_key: Some("No document"), text: false, footer: false },
        PanelCase {
            name: "brushes",
            group: Group::Brushes,
            tab: 0,
            key: "Create new brush from the current settings",
            empty_key: None,
            text: false,
            footer: true,
        },
        PanelCase { name: "character", group: Group::Character, tab: 0, key: "Character", empty_key: Some("Character"), text: true, footer: false },
        PanelCase { name: "paragraph", group: Group::Character, tab: 1, key: "Paragraph", empty_key: Some("Paragraph"), text: true, footer: false },
        PanelCase { name: "navigator", group: Group::Navigator, tab: 0, key: "Zoom In", empty_key: Some("No document"), text: false, footer: false },
        PanelCase { name: "histogram", group: Group::Navigator, tab: 1, key: "Channel:", empty_key: Some("No document"), text: false, footer: false },
        PanelCase { name: "info", group: Group::Navigator, tab: 2, key: "Doc:", empty_key: Some("No document"), text: false, footer: false },
        PanelCase { name: "actions", group: Group::History, tab: 1, key: "Play selection", empty_key: Some("Actions"), text: false, footer: true },
        PanelCase {
            name: "layer-comps",
            group: Group::History,
            tab: 2,
            key: "Create new layer comp",
            empty_key: Some("No document"),
            text: false,
            footer: true,
        },
        PanelCase { name: "channels", group: Group::Layers, tab: 1, key: "Create new channel", empty_key: Some("No document"), text: false, footer: true },
        PanelCase { name: "paths", group: Group::Layers, tab: 2, key: "Create new path", empty_key: Some("No document"), text: false, footer: true },
    ]
}

/// Pro's Color group opens on the picker field and puts Swatches second; the swatch, gradient
/// and pattern browsers are the same tabs in every theme, and the picker's HSB card lives on the
/// other themes' Color tab.
fn color_cases(pro: bool) -> Vec<PanelCase> {
    let mut v = vec![
        PanelCase { name: "swatches", group: Group::Color, tab: if pro { 1 } else { 0 }, key: "Create new swatch", empty_key: None, text: false, footer: true },
        PanelCase { name: "gradients", group: Group::Color, tab: 2, key: "Create new gradient", empty_key: None, text: false, footer: true },
        PanelCase { name: "patterns", group: Group::Color, tab: 3, key: "Create new pattern from the selection", empty_key: None, text: false, footer: true },
    ];
    if !pro {
        v.push(PanelCase { name: "color", group: Group::Color, tab: 1, key: "Hue", empty_key: None, text: false, footer: false });
    }
    v
}

fn assert_body(case: &PanelCase, theme: ThemeKind, scale: f32, width: f32) {
    let mut h = draw_body(app(false, case.group, case.tab, case.text), theme, scale, width, case.group, case.tab);
    let (body, size) = body_metrics(&h, case.group, case.tab);
    assert!(body.width() > 0.0 && body.height() > 0.0, "{} at {theme:?} ×{scale}: the body has room", case.name);
    assert!(size.x <= body.width() + 0.5, "{} at {theme:?} ×{scale}: no horizontal overflow ({size:?} in {body:?})", case.name);
    let key = settle(&mut h, case.key).unwrap_or_else(|| {
        let labels: Vec<String> = h.query_all_by_label_contains("").filter_map(|n| n.accesskit_node().label()).collect();
        panic!("{} at {theme:?} ×{scale}: {} is present; labels {labels:?}", case.name, case.key)
    });
    let bottom_slack = if case.footer { 9.0 } else { 1.0 };
    assert!(
        key.left() >= body.left() - 1.0 && key.right() <= body.right() + 1.0 && key.top() >= body.top() - 1.0 && key.bottom() <= body.bottom() + bottom_slack,
        "{} at {theme:?} ×{scale}: {} is not clipped ({key:?} in {body:?})",
        case.name,
        case.key
    );

    if let Some(empty) = case.empty_key {
        let h = draw_body(app(true, case.group, case.tab, false), theme, scale, width, case.group, case.tab);
        let (body, size) = body_metrics(&h, case.group, case.tab);
        assert!(size.x <= body.width() + 0.5, "{} empty at {theme:?} ×{scale}: no horizontal overflow ({size:?} in {body:?})", case.name);
        let label = node_starts(&h, empty);
        assert!(inside(body, label), "{} empty at {theme:?} ×{scale}: {empty} is visible ({label:?} in {body:?})", case.name);
    }
}

#[test]
fn priority_dock_bodies_fit_a_340_column_in_every_theme() {
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        for case in cases(pro).into_iter().chain(color_cases(pro)) {
            assert_body(&case, theme, 1.0, 340.0);
        }
    }
}

/// The dock can be dragged down to its 250-point minimum: cards, fields and footers must still
/// fit without a horizontal scrollbar.
#[test]
fn dock_bodies_fit_a_250_column_in_every_theme() {
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        for case in cases(pro).into_iter().chain(color_cases(pro)) {
            assert_body(&case, theme, 1.0, 250.0);
        }
    }
}

#[test]
fn priority_dock_bodies_fit_at_125_percent_ui_scale() {
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        for case in cases(pro).into_iter().chain(color_cases(pro)) {
            assert_body(&case, theme, 1.25, 340.0);
        }
    }
}

/// The dock frame around Layers: the group sits in the window, its tab strip and menu are inside
/// it, and the sticky footer's buttons end at the body's bottom edge instead of being cut.
fn assert_dock_frame(theme: ThemeKind, scale: f32) {
    let h = draw_dock(app(false, Group::Layers, 0, false), theme, scale);
    let content = h.ctx.content_rect();
    let (_, group_rect) = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == Group::Layers).expect("the Layers group is drawn");
    assert!(inside(content, group_rect), "Layers at {theme:?} ×{scale}: the group stays in the window ({group_rect:?} in {content:?})");

    let strip = crate::dock::last_strips(&h.ctx).into_iter().find(|s| s.group == Group::Layers).expect("the Layers strip is drawn");
    assert!(inside(group_rect, strip.area), "Layers at {theme:?} ×{scale}: the tab strip is inside the group");
    assert!(inside(group_rect, strip.menu), "Layers at {theme:?} ×{scale}: the panel menu is inside the group");
    assert!(strip.tabs.iter().any(|(tab, _)| *tab == 0), "Layers at {theme:?} ×{scale}: the Layers tab is on the strip");
    for (_, rect) in &strip.tabs {
        assert!(inside(group_rect, *rect), "Layers at {theme:?} ×{scale}: every visible tab is inside the group");
    }

    let body = Rect::from_min_max(pos2(group_rect.left(), strip.area.bottom()), group_rect.max);
    assert!(body.height() > 100.0, "Layers at {theme:?} ×{scale}: the body has room ({body:?})");
    let action = node_starts(&h, "Create a new layer");
    assert!(inside(body, action), "Layers at {theme:?} ×{scale}: the primary action is fully in the body ({action:?} in {body:?})");
    assert!(
        action.bottom() >= body.bottom() - 40.0,
        "Layers at {theme:?} ×{scale}: the footer sits at the bottom edge, not cut above it ({action:?} in {body:?})"
    );
}

/// The panels whose primary control is a footer button: in the real dock their action row must
/// stay above the group's bottom edge, not just inside a bare body harness.
#[test]
fn dock_tabs_keep_their_action_rows_in_bounds() {
    const NAMES: [&str; 7] = ["swatches", "gradients", "brushes", "actions", "layer-comps", "channels", "paths"];
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        let all: Vec<PanelCase> = cases(pro).into_iter().chain(color_cases(pro)).collect();
        for name in NAMES {
            let case = all.iter().find(|c| c.name == name).expect("a QA case");
            let h = draw_dock(app(false, case.group, case.tab, case.text), theme, 1.25);
            let (_, group_rect) = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == case.group).expect("the group is drawn");
            let key = node_starts(&h, case.key);
            assert!(inside(group_rect, key), "{} at {theme:?} ×1.25: {} stays in the group ({key:?} in {group_rect:?})", case.name, case.key);
        }
    }
}

#[test]
fn dock_frame_keeps_the_strip_and_footer_in_bounds() {
    for theme in THEMES {
        assert_dock_frame(theme, 1.0);
        assert_dock_frame(theme, 1.25);
    }
}

/// Without a document, the dock's empty state is what a new user sees: the card must be on screen
/// in every theme, and its guidance must stay inside the panel.
#[test]
fn dock_empty_states_are_visible_in_every_theme() {
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        for case in cases(pro).into_iter().filter(|c| c.empty_key.is_some() && (pro || c.group != Group::Properties)) {
            let h = draw_dock(app(true, case.group, case.tab, false), theme, 1.0);
            let (_, group_rect) = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == case.group).expect("the group is drawn");
            let key = case.empty_key.unwrap_or_default();
            let label = node_starts(&h, key);
            assert!(inside(group_rect, label), "{} empty at {theme:?}: {key} is inside the group ({label:?} in {group_rect:?})", case.name);
        }
    }
}

#[test]
#[ignore = "writes dock panel screenshots through wgpu for visual inspection"]
fn write_dock_panel_screenshots() {
    std::fs::create_dir_all(output_dir()).expect("screenshot dir");
    for theme in THEMES {
        let pro = Tokens::for_kind(theme).pro;
        for scale in [1.0, 1.25] {
            let suffix = if scale > 1.0 { "-125" } else { "" };
            for case in cases(pro).into_iter().chain(color_cases(pro)) {
                let mut h = draw_body(app(false, case.group, case.tab, case.text), theme, scale, 340.0, case.group, case.tab);
                settle(&mut h, case.key);
                save_screenshot(&mut h, &format!("{}-{}{suffix}", case.name, theme.id()));
                if case.empty_key.is_some() {
                    let mut h = draw_body(app(true, case.group, case.tab, false), theme, scale, 340.0, case.group, case.tab);
                    save_screenshot(&mut h, &format!("{}-empty-{}{suffix}", case.name, theme.id()));
                }
            }
            let mut h = draw_dock(app(false, Group::Layers, 0, false), theme, scale);
            save_screenshot(&mut h, &format!("dock-layers-{}{suffix}", theme.id()));
        }
    }
}

fn save_screenshot(h: &mut Harness<'static, PhotocraftApp>, name: &str) {
    h.ctx.request_repaint();
    h.run_steps(2);
    let image = h.render().expect("wgpu render");
    let pic = photocraft_codecs::Image::from_u8(image.width(), image.height(), photocraft_codecs::ChannelLayout::Rgba, image.as_raw().clone())
        .expect("wrap the rendered pixels");
    let png = photocraft_codecs::encode(&pic, photocraft_codecs::Format::Png, &Default::default()).expect("encode png");
    std::fs::create_dir_all(output_dir()).expect("screenshot dir");
    std::fs::write(output_dir().join(format!("{name}.png")), png).expect("write png");
}

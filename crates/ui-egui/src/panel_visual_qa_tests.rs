//! Offscreen visual QA for the redesigned floating panels — Retouch AI, Basic Color, Color Presets
//! and Face Reshape — in light themes at 100% and 125% UI scale.
//!
//! The assertions run with the normal `--lib` suite: every panel stays inside a 1440×900 window,
//! no scroll body overflows horizontally, the header/primary actions do not overlap, and every
//! required control is present and can be scrolled into view. The ignored writers render the same
//! states through wgpu to PNGs for a human look:
//! `cargo test -p photocraft-ui-egui --lib panel_visual_qa -- --ignored`.

use egui::{Rect, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::{ThemeKind, Tokens};

/// The light themes a panel can open in, including the low-contrast ones where a hard-coded
/// dark-theme colour would disappear.
const LIGHT_THEMES: [ThemeKind; 4] = [ThemeKind::ProLight, ThemeKind::ProLightGray, ThemeKind::StudioLight, ThemeKind::SoftLight];

/// Where the screenshot writers put their PNGs.
fn output_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/private/var/folders/30/m6f55_095bs3mldd6bmn06tm0000gn/T/opencode/ui-light")
}

fn app_with_document() -> PhotocraftApp {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).expect("new document");
    session.execute("layer.newAdjustmentLayer.basicColor", json!({})).expect("basic color layer");
    PhotocraftApp::new(session, crate::Services::default())
}

fn app_no_document() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn draw(app: PhotocraftApp, scale: f32, theme: ThemeKind, show: PanelDraw) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            // The app docks floating panels over the canvas; the harness has no canvas pass.
            app.last_canvas_rect = ctx.content_rect();
            show(app, &ctx);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if scale != 1.0 {
        // Interface › UI Scale: the physical window stays 1440×900 and the content area becomes
        // 1440/scale × 900/scale points, exactly as `prefs_ui` applies it.
        h.ctx.set_zoom_factor(scale);
    }
    h.run_steps(4);
    h
}

fn basic(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.panels.basic_color = true;
    crate::basic_color_panel::window(app, ctx);
}

fn retouch(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.panels.retouch_ai = true;
    crate::retouch_ai_panel::window(app, ctx);
}

fn presets(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.panels.color_presets = true;
    crate::color_presets_panel::window(app, ctx);
}

fn face(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.panels.face_reshape = true;
    crate::face_reshape_panel::window(app, ctx);
}

/// A panel's draw function (the window function under test).
type PanelDraw = fn(&mut PhotocraftApp, &egui::Context);

const PANELS: [(&str, PanelDraw); 4] = [("basic-color", basic), ("retouch-ai", retouch), ("color-presets", presets), ("face-reshape", face)];

fn panel_rect(h: &Harness<'_, PhotocraftApp>, id: &str) -> Rect {
    h.ctx.memory(|mem| mem.area_rect(egui::Id::new(id))).unwrap_or_else(|| panic!("the {id} panel is open"))
}

fn body_metrics(h: &Harness<'_, PhotocraftApp>, id: &str) -> (Rect, Vec2) {
    h.ctx.data(|d| d.get_temp::<(Rect, Vec2)>(egui::Id::new(id).with("body"))).unwrap_or_else(|| panic!("the {id} body recorded its metrics"))
}

fn node_rect(h: &Harness<'_, PhotocraftApp>, label: &str) -> Rect {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label} is present")).rect()
}

/// The smallest node carrying `label`: a window's accessible name (the whole panel) also matches
/// its title, so the title row checks need the label's own rect, not the window's.
fn label_rect(h: &Harness<'_, PhotocraftApp>, label: &str) -> Rect {
    h.query_all_by_label(label)
        .map(|node| node.rect())
        .min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
        .unwrap_or_else(|| panic!("{label} is present"))
}

/// Scroll `label` into the panel's scroll viewport and return its rect.
fn scroll_to(h: &mut Harness<'_, PhotocraftApp>, id: &str, label: &str) -> Rect {
    let node = h.query_all_by_label(label).next().unwrap_or_else(|| panic!("{label} is present"));
    node.scroll_to_me();
    h.run_steps(4);
    let rect = node_rect(h, label);
    let (body, _) = body_metrics(h, id);
    assert!(rect.top() >= body.top() - 1.0 && rect.bottom() <= body.bottom() + 1.0, "{label} scrolls fully into {id}: {rect:?} in {body:?}");
    assert!(rect.left() >= body.left() - 1.0 && rect.right() <= body.right() + 1.0, "{label} is not clipped horizontally: {rect:?} in {body:?}");
    rect
}

/// The panel sits inside the window, its body never overflows horizontally, and its header
/// controls stay clear of each other at this UI scale.
fn assert_panel_layout(name: &str, show: PanelDraw, scale: f32, theme: ThemeKind) {
    let h = draw(app_with_document(), scale, theme, show);
    let content = h.ctx.content_rect();
    let panel = panel_rect(&h, name);
    assert!(content.contains_rect(panel), "{name} at {theme:?} ×{scale}: {panel:?} stays inside {content:?}");
    let (body, size) = body_metrics(&h, name);
    assert!(size.x <= body.width() + 0.5, "{name} at {theme:?} ×{scale}: no horizontal overflow ({size:?} in {body:?})");
    assert!(body.width() > 0.0 && body.height() > 0.0, "{name} at {theme:?} ×{scale}: the body has room");

    // Header: the title, the shortcuts button and the close box do not overlap.
    let title = match name {
        "basic-color" => "Basic Color",
        "retouch-ai" => "Retouch AI",
        "color-presets" => "Color Presets",
        _ => crate::face_reshape_panel::entry_label(),
    };
    let title = label_rect(&h, title);
    let shortcuts = node_rect(&h, "Keyboard Shortcuts…");
    let close = node_rect(&h, "Close");
    for (a, ra, b, rb) in [("title", title, "shortcuts", shortcuts), ("title", title, "close", close), ("shortcuts", shortcuts, "close", close)] {
        assert!(!ra.intersects(rb), "{name} at {theme:?} ×{scale}: {a} and {b} do not overlap ({ra:?} vs {rb:?})");
    }
    for (label, rect) in [("title", title), ("shortcuts", shortcuts), ("close", close)] {
        assert!(panel.contains_rect(rect), "{name} at {theme:?} ×{scale}: the header {label} is inside the panel ({rect:?} in {panel:?})");
    }
}

#[test]
fn light_theme_panels_fit_at_1440_by_900() {
    for theme in LIGHT_THEMES {
        for (name, show) in PANELS {
            assert_panel_layout(name, show, 1.0, theme);
        }
    }
}

#[test]
fn light_theme_panels_fit_at_125_percent_ui_scale() {
    for theme in LIGHT_THEMES {
        for (name, show) in PANELS {
            assert_panel_layout(name, show, 1.25, theme);
        }
    }
}

fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let lin = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let lum = |c: egui::Color32| 0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b());
    let (hi, lo) = if lum(a) > lum(b) { (lum(a), lum(b)) } else { (lum(b), lum(a)) };
    (hi + 0.05) / (lo + 0.05)
}

/// `fg` composited over `bg` with `fg`'s alpha (sRGB channels, no gamma re-encoding).
fn over(fg: egui::Color32, bg: egui::Color32) -> egui::Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1.0 - a)).round() as u8;
    egui::Color32::from_rgb(mix(fg.r(), bg.r()), mix(fg.g(), bg.g()), mix(fg.b(), bg.b()))
}

/// The empty-state hint is the one line every panel shows without a document; it must keep AA
/// contrast on the panel card in every light theme (the amber `warning` token is 1.7:1 on ProLight).
#[test]
fn empty_state_hints_and_body_text_meet_aa_contrast_in_light_themes() {
    for theme in LIGHT_THEMES {
        let t = Tokens::for_kind(theme);
        for surface in [t.card, t.field] {
            assert!(contrast(t.text, surface) >= 7.0, "{theme:?}: body text on {surface:?}");
            assert!(contrast(t.text_dim, surface) >= 4.5, "{theme:?}: the empty-state hint and notes on {surface:?}");
        }
        // The error note draws its message in body text over a soft danger tint; the text stays
        // readable and the tint carries the error.
        assert!(contrast(t.text, t.card) >= 7.0, "{theme:?}: error text on the tinted card");
        // Preset card captions are small white text on `scrim`; the light themes paint it twice
        // (`paint_item`), so the strip must still clear AA over the palest card.
        let mut strip = t.card;
        for _ in 0..if t.dark() { 1 } else { 2 } {
            strip = over(t.scrim, strip);
        }
        assert!(contrast(egui::Color32::from_rgb(245, 245, 245), strip) >= 4.5, "{theme:?}: preset card captions on {strip:?}");
    }
}

#[test]
fn retouch_lists_all_14_tools_and_primary_actions_at_125_percent() {
    for theme in LIGHT_THEMES {
        let mut h = draw(app_with_document(), 1.25, theme, retouch);
        let panel = panel_rect(&h, "retouch-ai");
        assert!(panel.contains_rect(node_rect(&h, "Run Selected")), "{theme:?}: the primary Run action is fully in view");
        assert!(panel.contains_rect(node_rect(&h, "Analyze")), "{theme:?}: Analyze is fully in view");
        for spec in crate::retouch_ai_panel::TOOLS {
            let label = crate::retouch_ai_panel::tool_label(spec.id);
            let rect = scroll_to(&mut h, "retouch-ai", label);
            assert!(panel.contains_rect(rect), "{theme:?}: {label} is inside the panel");
        }
    }
}

#[test]
fn basic_color_lists_ten_sliders_and_eight_mixer_bands_at_125_percent() {
    const SLIDERS: &[&str] = &["Temperature", "Tint", "Exposure", "Contrast", "Highlights", "Shadows", "Whites", "Blacks", "Vibrance", "Saturation"];
    const BANDS: &[&str] = &["Red", "Orange", "Yellow", "Green", "Mint", "Blue", "Purple", "Pink"];
    for theme in LIGHT_THEMES {
        let mut h = draw(app_with_document(), 1.25, theme, basic);
        let panel = panel_rect(&h, "basic-color");
        for name in SLIDERS {
            let rect = scroll_to(&mut h, "basic-color", name);
            assert!(panel.contains_rect(rect), "{theme:?}: {name} is inside the panel");
        }
        // The mixer is the second page of the same panel: eight bands, one per hue centre.
        h.get_by_label("Color Mixer").click();
        h.run_steps(2);
        for name in BANDS {
            let rect = scroll_to(&mut h, "basic-color", name);
            assert!(panel.contains_rect(rect), "{theme:?}: mixer band {name} is inside the panel");
        }
    }
}

#[test]
fn color_presets_has_search_import_cards_and_batch_at_125_percent() {
    for theme in LIGHT_THEMES {
        let mut h = draw(app_with_document(), 1.25, theme, presets);
        let panel = panel_rect(&h, "color-presets");
        // The search box is the topmost text field; the hint text itself is a placeholder.
        let search =
            h.query_all_by_role(egui::accesskit::Role::TextInput).map(|node| node.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("the search field");
        assert!(panel.contains_rect(search), "{theme:?}: the search field is inside the panel ({search:?})");
        for label in ["Import LUT folder…", "Save current look", "Warm Portrait", "Cool Soft", "Punchy", "Faded Film", "Black & White"] {
            assert!(panel.contains_rect(node_rect(&h, label)), "{theme:?}: {label} is inside the panel");
        }
        for label in ["Input folder", "Output folder", "Format", "Include subfolders", "Keep files if cancelled, so I can resume", "Run"] {
            let rect = scroll_to(&mut h, "color-presets", label);
            assert!(panel.contains_rect(rect), "{theme:?}: {label} is inside the panel");
        }
        assert!(h.query_all_by_label("Browse…").count() >= 2, "{theme:?}: both folder rows have a Browse button");
    }
}

#[test]
fn face_reshape_reaches_all_68_sliders_and_actions_at_125_percent() {
    const GROUPS: [(&str, usize); 6] =
        [("Face & Head Shape", 17), ("Eyes & Iris", 13), ("Eyebrows", 7), ("Nose", 11), ("Mouth & Lips", 12), ("Proportions & Neck", 8)];
    for theme in LIGHT_THEMES {
        let mut h = draw(app_with_document(), 1.25, theme, face);
        let panel = panel_rect(&h, "face-reshape");
        for label in ["Compare", "Reset All", "Reset Group", "Apply"] {
            assert!(panel.contains_rect(node_rect(&h, label)), "{theme:?}: the sticky {label} action is visible");
        }
        let mut total = 0;
        for (index, (header, count)) in GROUPS.into_iter().enumerate() {
            // The accordion is the user's path; set the open group the same way a header click
            // does, then verify its row count.
            h.state_mut().ui.face_reshape.active_group = index;
            h.run_steps(3);
            let sliders = h.query_all_by_role(egui::accesskit::Role::Slider).count();
            assert_eq!(sliders, count, "{theme:?}: {header} shows its {count} sliders");
            let (body, _) = body_metrics(&h, "face-reshape");
            for role in [egui::accesskit::Role::Slider, egui::accesskit::Role::SpinButton] {
                for node in h.query_all_by_role(role) {
                    let rect = node.rect();
                    assert!(
                        rect.left() >= body.left() - 1.0 && rect.right() <= body.right() + 1.0,
                        "{theme:?}: {header} {role:?} is not clipped horizontally: {rect:?} in {body:?}"
                    );
                }
            }
            total += sliders;
        }
        assert_eq!(total, 68, "{theme:?}: every engine slider is reachable through the accordions");
        // A header click switches to that group too.
        h.get_by_label("Eyes & Iris").click();
        h.run_steps(4);
        assert_eq!(h.state().ui.face_reshape.active_group, 1, "{theme:?}: clicking a group header opens it");
    }
}

/// Every primary action row keeps its buttons clear of each other at 125%: the Retouch and Face
/// action pairs, the Color Presets save/import/search row, and the Basic Color tabs.
#[test]
fn primary_action_rows_do_not_overlap_at_125_percent() {
    for theme in LIGHT_THEMES {
        let h = draw(app_with_document(), 1.25, theme, retouch);
        let analyze = node_rect(&h, "Analyze");
        let run = node_rect(&h, "Run Selected");
        assert!(!analyze.intersects(run), "{theme:?}: Retouch Analyze and Run do not overlap ({analyze:?} vs {run:?})");

        let h = draw(app_with_document(), 1.25, theme, face);
        let compare = node_rect(&h, "Compare");
        let reset_all = node_rect(&h, "Reset All");
        let reset_group = node_rect(&h, "Reset Group");
        let apply = node_rect(&h, "Apply");
        for (a, ra, b, rb) in [
            ("Compare", compare, "Reset All", reset_all),
            ("Reset All", reset_all, "Reset Group", reset_group),
            ("Compare", compare, "Reset Group", reset_group),
            ("Compare", compare, "Apply", apply),
        ] {
            assert!(!ra.intersects(rb), "{theme:?}: Face Reshape {a} and {b} do not overlap ({ra:?} vs {rb:?})");
        }

        let h = draw(app_with_document(), 1.25, theme, basic);
        let basic_tab = node_rect(&h, "Basic");
        let mixer_tab = node_rect(&h, "Color Mixer");
        assert!(!basic_tab.intersects(mixer_tab), "{theme:?}: Basic Color's two tabs do not overlap");
    }
}

#[test]
#[ignore = "writes light-theme panel screenshots through wgpu for visual inspection"]
fn write_light_theme_panel_screenshots() {
    for theme in LIGHT_THEMES {
        for scale in [1.0, 1.25] {
            for (name, show) in PANELS {
                let mut h = draw(app_with_document(), scale, theme, show);
                save_screenshot(&mut h, &format!("{name}-{}-{}", theme.id(), (scale * 100.0) as u32));
            }
        }
    }
}

#[test]
#[ignore = "writes the Basic Color mixer page and a scrolled Retouch tool list"]
fn write_panel_page_screenshots() {
    for theme in [ThemeKind::ProLight, ThemeKind::StudioLight, ThemeKind::SoftLight] {
        let mut h = draw(app_with_document(), 1.0, theme, basic);
        h.get_by_label("Color Mixer").click();
        h.run_steps(2);
        save_screenshot(&mut h, &format!("basic-color-mixer-{}", theme.id()));

        let mut h = draw(app_with_document(), 1.0, theme, retouch);
        h.get_by_label("Color Match").scroll_to_me();
        h.run_steps(2);
        save_screenshot(&mut h, &format!("retouch-ai-scrolled-{}", theme.id()));
    }
}

#[test]
#[ignore = "writes the empty-state hint in light themes"]
fn write_no_document_screenshots() {
    for theme in LIGHT_THEMES {
        for scale in [1.0, 1.25] {
            for (name, show) in PANELS {
                let mut h = draw(app_no_document(), scale, theme, show);
                save_screenshot(&mut h, &format!("{name}-nodoc-{}-{}", theme.id(), (scale * 100.0) as u32));
            }
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

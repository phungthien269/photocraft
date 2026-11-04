//! Brand recognition and first-run guidance: the Home lockup, the primary action, the Quick start
//! card, the empty Recent guidance, the About lockup with its palette, and the shared empty-state
//! watermark. The assertions run offscreen in every theme and at 125% UI scale; the ignored writer
//! renders the same states to PNGs for a human look:
//! `cargo test -p photocraft-ui-egui --lib brand_ui -- --ignored`.

use egui::{Color32, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::PhotocraftApp;
use crate::theme::{BRAND_BLUE, BRAND_INK, BRAND_PAPER, ThemeKind, Tokens};

/// Where the screenshot writer puts its PNGs.
fn output_dir() -> std::path::PathBuf {
    std::path::PathBuf::from("/private/var/folders/30/m6f55_095bs3mldd6bmn06tm0000gn/T/opencode/ui-brand")
}

fn app(recent: usize) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.recent_files = (0..recent).map(|i| format!("/tmp/photocraft-brand-{i}.png")).collect();
    app
}

fn fonts_ready(ui: &egui::Ui) -> bool {
    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        return true;
    }
    ui.ctx().request_repaint();
    false
}

/// The Home screen with no document, filling a 1440×900 window.
fn draw_home(app: PhotocraftApp, theme: ThemeKind, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !fonts_ready(ui) {
                return;
            }
            ui.painter().rect_filled(ui.max_rect(), 0.0, Tokens::get(ui.ctx()).canvas);
            ui.scope(|ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if scale != 1.0 {
        h.ctx.set_zoom_factor(scale);
    }
    h.run_steps(6);
    h
}

/// The About window on its default tab, over the Home screen in a 1440×900 window. Drawn through
/// `build_ui_state` rather than `build_eframe` so 125% actually scales the dialog for the check.
fn draw_about(theme: ThemeKind, scale: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !fonts_ready(ui) {
                return;
            }
            ui.painter().rect_filled(ui.max_rect(), 0.0, Tokens::get(ui.ctx()).canvas);
            ui.scope(|ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, ui.ctx());
        },
        app(0),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    if scale != 1.0 {
        h.ctx.set_zoom_factor(scale);
    }
    h.state_mut().ui.open_dialog(crate::state::DialogKind::About, Default::default());
    h.run_steps(6);
    h
}

/// The shared dock empty state in a card `width` points wide.
fn draw_empty_state(width: f32, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(width, 220.0)).build_ui_state(
        move |ui, _app: &mut PhotocraftApp| {
            if !fonts_ready(ui) {
                return;
            }
            ui.painter().rect_filled(ui.max_rect(), 0.0, Tokens::get(ui.ctx()).card);
            ui.set_width(width);
            crate::widgets::empty_state(ui, "clock", "Recent", "Open an image to start");
        },
        app(0),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(5);
    h
}

fn label(h: &Harness<'_, PhotocraftApp>, text: &str) -> Rect {
    h.query_all_by_label(text).map(|n| n.rect()).next().unwrap_or_else(|| panic!("{text} is on screen"))
}

fn label_contains(h: &Harness<'_, PhotocraftApp>, text: &str) -> Rect {
    h.query_all_by_label_contains(text).map(|n| n.rect()).next().unwrap_or_else(|| panic!("{text} is on screen"))
}

/// The smallest node whose label contains `text`: the Open… button and the Quick start row share
/// the text, and the button is the compact one.
fn smallest_contains(h: &Harness<'_, PhotocraftApp>, text: &str) -> Rect {
    h.query_all_by_label_contains(text)
        .map(|n| n.rect())
        .min_by(|a, b| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
        .unwrap_or_else(|| panic!("{text} is on screen"))
}

fn inside(outer: Rect, inner: Rect) -> bool {
    inner.left() >= outer.left() - 1.0 && inner.right() <= outer.right() + 1.0 && inner.top() >= outer.top() - 1.0 && inner.bottom() <= outer.bottom() + 1.0
}

fn content_rect(h: &Harness<'_, PhotocraftApp>) -> Rect {
    h.ctx.content_rect()
}

#[test]
fn home_shows_the_lockup_and_a_primary_action_in_every_theme() {
    for theme in ThemeKind::ALL {
        let h = draw_home(app(3), theme, 1.0);
        let card = crate::canvas::home_card_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?}: the Home card is drawn"));
        assert!(inside(content_rect(&h), card), "{theme:?}: the Home card stays in the window ({card:?})");
        let mark = crate::brand::mark_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?}: the brand mark is painted"));
        assert!((mark.width() - 48.0).abs() < 0.5 && (mark.height() - 48.0).abs() < 0.5, "{theme:?}: the Home mark is 48 pt ({mark:?})");
        assert!(inside(card, mark), "{theme:?}: the mark is inside the Home card ({mark:?} in {card:?})");
        let wordmark = label(&h, "PhotoCraft");
        assert!(inside(card, wordmark), "{theme:?}: the wordmark is inside the Home card ({wordmark:?})");
        assert!(inside(card, label(&h, "open source")), "{theme:?}: the open-source chip is inside the Home card");
        let primary = label_contains(&h, "New document");
        let secondary = smallest_contains(&h, "Open…");
        assert!(inside(card, primary), "{theme:?}: the primary action is inside the Home card ({primary:?} in {card:?})");
        assert!(primary.width() > secondary.width(), "{theme:?}: the primary action is wider than Open ({:?} vs {:?})", primary.size(), secondary.size());
        assert!(primary.left() < card.center().x, "{theme:?}: the primary action sits at the card's start");
    }
}

#[test]
fn home_quick_start_offers_three_existing_actions_without_overflow() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 1.25] {
            let h = draw_home(app(3), theme, scale);
            let card = crate::canvas::home_card_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?} ×{scale}: the Home card is drawn"));
            assert!(inside(content_rect(&h), card), "{theme:?} ×{scale}: the Home card stays in the window ({card:?})");
            let title = label(&h, "Quick start");
            assert!(inside(card, title), "{theme:?} ×{scale}: the Quick start title is inside the Home card ({title:?} in {card:?})");
            for action in ["Open…", "Retouch AI", "Keyboard Shortcuts…"] {
                let row = label(&h, action);
                assert!(inside(card, row), "{theme:?} ×{scale}: {action} is inside the Home card ({row:?} in {card:?})");
                assert!(row.left() >= title.left() - 1.0, "{theme:?} ×{scale}: {action} lines up under Quick start");
            }
        }
    }
}

#[test]
fn home_recent_list_with_six_entries_fits_at_125_percent() {
    for theme in ThemeKind::ALL {
        let h = draw_home(app(6), theme, 1.25);
        let card = crate::canvas::home_card_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?}: the Home card is drawn"));
        assert!(inside(content_rect(&h), card), "{theme:?} ×1.25: six recent rows keep the card in the window ({card:?} in {:?})", content_rect(&h));
        assert!(label_contains(&h, "Recent").top() > card.center().y - 80.0, "{theme:?}: the Recent heading is above the file rows");
        for i in 0..6 {
            let name = format!("photocraft-brand-{i}.png");
            let row = label(&h, &name);
            assert!(inside(card, row), "{theme:?} ×1.25: recent row {i} is inside the Home card ({row:?} in {card:?})");
        }
        // The last row must not run into the links below it: the card keeps a gap under the list,
        // and the Discord button that follows stays inside the card too.
        let last = label(&h, "photocraft-brand-5.png");
        assert!(last.bottom() < card.bottom() - 30.0, "{theme:?} ×1.25: the list leaves room for the links ({last:?} in {card:?})");
        let discord = label_contains(&h, "Join us on Discord");
        assert!(inside(card, discord), "{theme:?} ×1.25: the Discord button stays in the Home card ({discord:?} in {card:?})");
    }
}

#[test]
fn home_empty_recent_shows_guidance_and_a_quiet_mark() {
    for theme in ThemeKind::ALL {
        let h = draw_home(app(0), theme, 1.0);
        let card = crate::canvas::home_card_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?}: the Home card is drawn"));
        let hint = label(&h, "Open an image to start");
        assert!(inside(card, hint), "{theme:?}: the empty Recent guidance is inside the Home card ({hint:?} in {card:?})");
        let discord = label_contains(&h, "Join us on Discord");
        assert!(inside(card, discord), "{theme:?}: the links stay inside the empty Home card ({discord:?} in {card:?})");
        let mark = crate::brand::watermark_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?}: the empty-state watermark is painted"));
        assert!((mark.width() - 40.0).abs() < 0.5, "{theme:?}: the empty-state watermark is 40 pt ({mark:?})");
        assert!(inside(card, mark), "{theme:?}: the watermark is inside the card ({mark:?} in {card:?})");
        assert!(
            mark.right() > hint.center().x && mark.top() < hint.top(),
            "{theme:?}: the watermark sits above and right of the guidance ({mark:?} vs {hint:?})"
        );
    }
}

#[test]
fn about_shows_the_lockup_palette_and_version_in_every_theme() {
    for theme in ThemeKind::ALL {
        for scale in [1.0, 1.25] {
            let h = draw_about(theme, scale);
            let mark = crate::brand::mark_rect(&h.ctx).unwrap_or_else(|| panic!("{theme:?} ×{scale}: the About mark is painted"));
            assert!((mark.width() - 64.0).abs() < 0.5 && (mark.height() - 64.0).abs() < 0.5, "{theme:?} ×{scale}: the About mark is 64 pt ({mark:?})");
            assert!(inside(content_rect(&h), mark), "{theme:?} ×{scale}: the About mark is on screen ({mark:?})");
            let wordmark = label(&h, "PhotoCraft");
            assert!(inside(content_rect(&h), wordmark), "{theme:?} ×{scale}: the About wordmark is on screen");
            assert!(label_contains(&h, "open-source").top() > wordmark.top(), "{theme:?} ×{scale}: the tagline sits under the wordmark");
            let chips: Vec<Rect> = ["#0B0B0C", "#EFE9DC", "#2F7BF5"].iter().map(|hex| label(&h, hex)).collect();
            for (hex, chip) in ["#0B0B0C", "#EFE9DC", "#2F7BF5"].iter().zip(&chips) {
                assert!(inside(content_rect(&h), *chip), "{theme:?} ×{scale}: palette chip {hex} is on screen ({chip:?})");
                assert!(chip.width() > 20.0, "{theme:?} ×{scale}: palette chip {hex} has its hex label");
            }
            assert!(chips[0].left() < chips[1].left() && chips[1].left() < chips[2].left(), "{theme:?} ×{scale}: the palette reads Ink, Paper, Blue");
            assert!(label_contains(&h, "Version ").top() > chips[0].bottom(), "{theme:?} ×{scale}: the version line sits under the palette");
            assert!(label(&h, "About").width() > 0.0, "{theme:?} ×{scale}: the About tab is untouched");
            assert!(label(&h, "Contributors").width() > 0.0, "{theme:?} ×{scale}: the Contributors tab is untouched");
            assert!(label(&h, "Models").width() > 0.0, "{theme:?} ×{scale}: the Models tab is untouched");
        }
    }
}

#[test]
fn empty_state_watermark_skips_narrow_cards_and_fits_wide_ones() {
    let wide = draw_empty_state(320.0, ThemeKind::Studio);
    let mark = crate::brand::watermark_rect(&wide.ctx).expect("a wide empty state carries the watermark");
    assert!((mark.width() - 40.0).abs() < 0.5, "the watermark is 40 pt ({mark:?})");
    assert!(inside(content_rect(&wide), mark), "the watermark stays in the wide card ({mark:?})");
    let narrow = draw_empty_state(180.0, ThemeKind::Studio);
    assert!(crate::brand::watermark_rect(&narrow.ctx).is_none(), "a narrow empty state skips the watermark");
}

fn channel(v: u8) -> f32 {
    let s = f32::from(v) / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn luminance(c: Color32) -> f32 {
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn over(fg: Color32, bg: Color32) -> Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| (f32::from(f) * a + f32::from(b) * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(mix(fg.r(), bg.r()), mix(fg.g(), bg.g()), mix(fg.b(), bg.b()))
}

#[test]
fn brand_palette_and_derived_accents_pass_contrast_in_every_theme() {
    for theme in ThemeKind::ALL {
        let t = Tokens::for_kind(theme);
        let b = t.brand();
        assert_eq!((b.ink, b.paper, b.blue), (BRAND_INK, BRAND_PAPER, BRAND_BLUE), "{theme:?}: the fixed palette is exact");
        for surface in [t.card, t.dock, t.field, t.chrome] {
            let ratio = contrast(b.accent, surface);
            assert!(ratio >= 4.5, "{theme:?}: brand accent text on {surface:?} reads at AA ({ratio:.2}:1)");
        }
        let icon_ratio = contrast(b.accent, over(b.soft, t.card));
        assert!(icon_ratio >= 3.0, "{theme:?}: brand icons on their soft wash read as UI components ({icon_ratio:.2}:1)");
        let label_ratio = contrast(b.accent, t.card);
        assert!(label_ratio >= 4.5, "{theme:?}: palette hex labels read at AA ({label_ratio:.2}:1)");
        let pill_ratio = contrast(b.ink, b.blue);
        assert!(pill_ratio >= 4.5, "{theme:?}: the exact blue chip carries ink text at AA ({pill_ratio:.2}:1)");
        let alpha = b.watermark.a();
        assert!(alpha > 0 && alpha <= 24, "{theme:?}: the watermark stays quiet (alpha {alpha})");
    }
}

#[test]
#[ignore = "writes Home and About screenshots through wgpu for visual inspection"]
fn write_brand_ui_screenshots() {
    std::fs::create_dir_all(output_dir()).expect("screenshot dir");
    const THEMES: [ThemeKind; 5] = [ThemeKind::Studio, ThemeKind::StudioLight, ThemeKind::SoftLight, ThemeKind::ProLight, ThemeKind::ProLightGray];
    for theme in THEMES {
        for scale in [1.0, 1.25] {
            let suffix = if scale > 1.0 { "-125" } else { "" };
            let mut h = draw_home(app(3), theme, scale);
            save_screenshot(&mut h, &format!("home-{}{suffix}", theme.id()));
            let mut h = draw_home(app(0), theme, scale);
            save_screenshot(&mut h, &format!("home-empty-{}{suffix}", theme.id()));
            let mut h = draw_about(theme, scale);
            save_screenshot(&mut h, &format!("about-{}{suffix}", theme.id()));
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

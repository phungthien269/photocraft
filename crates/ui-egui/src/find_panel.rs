//! Window › Find Panel…: a quick switcher for the dock's panels and the floating windows.
//!
//! The list is built from the two places the shell already knows: [`crate::panel_docking`]'s
//! registered panels (open or closed in the dock) and the floating `window.toggle.*` commands.
//! Nothing here duplicates a panel name or a toggle: labels come from those registries.
//!
//! The palette type-filters by panel name and group, remembers the panels used from it, shows an
//! `Open` badge for open ones, and opens/closes the selected panel with ↵. ↑/↓ move, Esc closes.

use egui::{Align, Align2, Color32, CornerRadius, Key, RichText, Sense, Stroke, vec2};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons};

/// One row in Find Panel….
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PanelEntry {
    /// Stable key for the recent list: `dock:<id>` or `toggle:<command id>`.
    pub key: String,
    pub label: String,
    pub group: String,
    /// The dock panel id, when this row is a dock panel.
    pub dock: Option<String>,
    /// The `window.toggle.*` command id, when this row is a floating window.
    pub toggle: Option<String>,
    pub open: bool,
}

/// The query and selection live in egui's temporary data, like the command palette's.
fn query_id() -> egui::Id {
    egui::Id::new("find-panel-query")
}
fn selection_id() -> egui::Id {
    egui::Id::new("find-panel-selection")
}
fn last_query_id() -> egui::Id {
    egui::Id::new("find-panel-last-query")
}

/// Window › Find Panel…: open the palette with a fresh query.
pub(crate) fn open(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.find_panel_open = true;
    // Only one search surface at a time.
    app.ui.palette_open = false;
    ctx.data_mut(|d| {
        d.insert_temp(query_id(), String::new());
        d.insert_temp(selection_id(), 0usize);
        d.insert_temp(last_query_id(), String::new());
    });
}

/// How many recent panels are remembered.
const RECENT_CAP: usize = 8;

/// Remember a row as the most recently used, de-duplicated and capped.
pub(crate) fn note_recent(app: &mut PhotocraftApp, key: &str) {
    app.ui.recent_panels.retain(|k| k != key);
    app.ui.recent_panels.insert(0, key.to_string());
    app.ui.recent_panels.truncate(RECENT_CAP);
}

/// Is this `window.toggle.*` command a dock panel's alias (so the dock row already lists it)?
fn dock_toggle(id: &str) -> bool {
    crate::panel_docking::registered().iter().any(|(panel, _, _)| crate::menus::panel_alias(&format!("window.panel.{panel}")) == Some(id))
}

/// Every panel the palette can switch, from the dock registry and the floating window commands.
pub(crate) fn entries(app: &PhotocraftApp) -> Vec<PanelEntry> {
    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
    let mut out: Vec<PanelEntry> = crate::panel_docking::registered()
        .iter()
        .map(|(id, label, group)| PanelEntry {
            key: format!("dock:{id}"),
            label: crate::i18n::tr(lang, label).to_string(),
            group: crate::i18n::tr(lang, group.label()).to_string(),
            dock: Some((*id).to_string()),
            toggle: None,
            open: crate::panel_docking::is_open(app, id),
        })
        .collect();
    for &(id, label, _, _) in crate::menus::UI_COMMANDS {
        if !id.starts_with("window.toggle.") || dock_toggle(id) {
            continue;
        }
        // Toolbar/options/dock are shell chrome, not panels a panel finder should offer.
        if matches!(id, "window.toggle.toolbar" | "window.toggle.options" | "window.toggle.dock") {
            continue;
        }
        // The floating windows are exactly the remaining window.toggle.* commands (Brush Settings,
        // Retouch AI, Basic Color, Color Presets, Face Reshape, and any added later).
        out.push(PanelEntry {
            key: format!("toggle:{id}"),
            label: crate::i18n::tr_id(lang, id, label).to_string(),
            group: crate::i18n::tr(lang, "Window").to_string(),
            dock: None,
            toggle: Some(id.to_string()),
            open: crate::menus::checked(app, id).unwrap_or(false),
        });
    }
    out
}

/// The rows matching `query`, best first. An empty query lists recents, then open panels, then the
/// rest alphabetically; a query scores names and groups (and gives recents a small nudge).
pub(crate) fn ranked(entries: &[PanelEntry], query: &str, recents: &[String]) -> Vec<usize> {
    let q = query.trim();
    let mut rows: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, entry)| {
            let recent = recents.iter().position(|k| k == &entry.key);
            if q.is_empty() {
                let score = recent.map_or(0, |at| 10_000 - at as i32 * 10) + i32::from(entry.open) * 5;
                return Some((score, i));
            }
            let hay = format!("{} {} {}", entry.label, entry.group, entry.key);
            let mut score = crate::palette::fuzzy_score(q, &hay)?;
            let label = entry.label.trim_end_matches('…').trim();
            if label.eq_ignore_ascii_case(q) {
                score += 2_000;
            }
            if entry.group.eq_ignore_ascii_case(q) {
                score += 500;
            }
            if let Some(at) = recent {
                score += 60 - at as i32 * 5;
            }
            Some((score, i))
        })
        .collect();
    rows.sort_by(|(sa, a), (sb, b)| sb.cmp(sa).then_with(|| entries[*a].label.to_lowercase().cmp(&entries[*b].label.to_lowercase())));
    rows.into_iter().map(|(_, i)| i).collect()
}

/// The command that switches `entry`: close an open dock panel, reveal a closed one, or toggle a
/// floating window.
fn action(entry: &PanelEntry) -> (String, serde_json::Value) {
    if let Some(panel) = entry.dock.as_deref() {
        if entry.open {
            ("window.panel.close".to_string(), serde_json::json!({"panel": panel}))
        } else {
            ("window.panel.activate".to_string(), serde_json::json!({"panel": panel}))
        }
    } else {
        (entry.toggle.clone().unwrap_or_default(), serde_json::json!({}))
    }
}

fn close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    app.ui.find_panel_open = false;
    ctx.data_mut(|d| {
        d.insert_temp(query_id(), String::new());
        d.insert_temp(selection_id(), 0usize);
        d.insert_temp(last_query_id(), String::new());
    });
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.find_panel_open {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Esc belongs to the palette: consume it before other handlers read it.
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        close(app, ctx);
        return;
    }

    let width = (screen.width() - 48.0).clamp(280.0, 620.0);
    let top = screen.top() + 96.0;
    // Keep the card inside the 1440×900 window even at 125% UI scale.
    let list_height = (screen.bottom() - top - 176.0).clamp(120.0, 460.0);
    let mut scrim_clicked = false;
    egui::Area::new(egui::Id::new("find-panel-scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (rect, response) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(rect, 0.0, Color32::from_black_alpha(110));
        scrim_clicked = response.clicked();
    });
    if scrim_clicked {
        close(app, ctx);
        return;
    }

    let entries = entries(app);
    let recents = app.ui.recent_panels.clone();
    let mut chosen: Option<PanelEntry> = None;
    let mut q: String = ctx.data(|d| d.get_temp(query_id()).unwrap_or_default());
    let mut selected: usize = ctx.data(|d| d.get_temp(selection_id()).unwrap_or(0));

    egui::Area::new(egui::Id::new("find-panel")).order(egui::Order::Tooltip).pivot(Align2::CENTER_TOP).fixed_pos(egui::pos2(screen.center().x, top)).show(
        ctx,
        |ui| {
            egui::Frame::NONE
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(width);
                    // Search field: a soft inset card with the icon and the effective shortcut.
                    egui::Frame::NONE
                        .fill(t.field)
                        .stroke(Stroke::new(1.0, t.field_border))
                        .corner_radius(CornerRadius::same(t.radius as u8))
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                icons::paint(ui, r, "search", 16.0, t.text_dim);
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut q)
                                        .hint_text(tl!("Search panels…"))
                                        .frame(egui::Frame::NONE)
                                        .font(egui::FontId::proportional(15.0))
                                        .desired_width(width - 92.0),
                                );
                                if !response.has_focus() {
                                    response.request_focus();
                                }
                                if let Some(sc) = crate::shortcuts::shortcut_label(app, "window.findPanel") {
                                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                        ui.label(RichText::new(sc).font(egui::FontId::proportional(11.5)).color(t.text_faint));
                                    });
                                }
                            });
                        });
                    ui.add_space(8.0);
                    let previous_query: String = ui.data(|d| d.get_temp(last_query_id()).unwrap_or_default());
                    if q != previous_query {
                        selected = 0;
                    }
                    let order = ranked(&entries, &q, &recents);
                    let (down, up, enter) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                            i.consume_key(egui::Modifiers::NONE, Key::Enter),
                        )
                    });
                    if down {
                        selected = (selected + 1).min(order.len().saturating_sub(1));
                    }
                    if up {
                        selected = selected.saturating_sub(1);
                    }
                    selected = selected.min(order.len().saturating_sub(1));
                    let moved = down || up;
                    let open_count = entries.iter().filter(|e| e.open).count();

                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tl!("Panels")).small().color(t.text_dim));
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(format!("{open_count} {}", tl!("Open"))).small().color(t.text_faint));
                        });
                    });
                    ui.add_space(4.0);
                    crate::widgets::hairline(ui);
                    ui.add_space(4.0);

                    let mut row_rects: Vec<egui::Rect> = Vec::with_capacity(order.len());
                    if order.is_empty() {
                        ui.allocate_ui_with_layout(vec2(width, 58.0), egui::Layout::left_to_right(Align::Center), |ui| {
                            ui.label(RichText::new(tl!("No matching panels")).color(t.text_faint));
                        });
                    } else {
                        egui::ScrollArea::vertical().id_salt("find-panel-list").max_height(list_height).auto_shrink([false, true]).show(ui, |ui| {
                            ui.set_width(width);
                            for (row, index) in order.iter().enumerate() {
                                let entry = &entries[*index];
                                let (rect, response) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
                                row_rects.push(rect);
                                let is_selected = row == selected;
                                if is_selected || response.hovered() {
                                    let fill = if is_selected { t.accent_soft } else { t.hover };
                                    ui.painter().rect_filled(rect.shrink2(vec2(2.0, 1.0)), CornerRadius::same(t.radius_sm as u8), fill);
                                }
                                let color = if entry.open { t.text } else { t.text_dim };
                                let text_right = rect.right() - 12.0 - if entry.open { 58.0 } else { 0.0 };
                                let clip = egui::Rect::from_min_max(egui::pos2(rect.left() + 12.0, rect.top()), egui::pos2(text_right, rect.bottom()));
                                let painter = ui.painter().with_clip_rect(clip);
                                painter.text(
                                    egui::pos2(rect.left() + 12.0, rect.center().y - 8.0),
                                    Align2::LEFT_CENTER,
                                    &entry.label,
                                    theme::medium(13.0),
                                    color,
                                );
                                painter.text(
                                    egui::pos2(rect.left() + 12.0, rect.center().y + 8.0),
                                    Align2::LEFT_CENTER,
                                    &entry.group,
                                    egui::FontId::proportional(11.0),
                                    t.text_faint,
                                );
                                if entry.open {
                                    draw_open_badge(ui, rect, &t);
                                }
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Button,
                                        true,
                                        is_selected,
                                        format!("{} — {}{}", entry.label, entry.group, if entry.open { " — Open" } else { "" }),
                                    )
                                });
                                if response.clicked() {
                                    selected = row;
                                    chosen = Some(entry.clone());
                                }
                                if is_selected && moved {
                                    ui.scroll_to_rect(rect, Some(Align::Center));
                                }
                            }
                        });
                    }

                    ui.add_space(6.0);
                    crate::widgets::hairline(ui);
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(crate::i18n::fmt(tl!("↑↓ navigate   {key} run   esc close"), &[("key", &crate::shortcuts::pretty("Enter"))]))
                            .small()
                            .color(t.text_faint),
                    );
                    // ↵ toggles the highlighted panel. Consume it so dialogs opened by the
                    // command do not read the same press as their OK button (#536).
                    if enter && let Some(entry) = order.get(selected).map(|i| entries[*i].clone()) {
                        chosen = Some(entry);
                        ui.ctx().input_mut(|i| {
                            i.consume_key(egui::Modifiers::NONE, Key::Enter);
                        });
                    }
                    ui.data_mut(|d| {
                        d.insert_temp(query_id(), q.clone());
                        d.insert_temp(selection_id(), selected);
                        d.insert_temp(last_query_id(), q.clone());
                    });
                    #[cfg(test)]
                    record_metrics(
                        ui.ctx(),
                        Metrics {
                            card: ui.min_rect(),
                            list_height,
                            rows: row_rects,
                            selected: order.get(selected).map(|i| entries[*i].key.clone()),
                            query: q.clone(),
                            empty: order.is_empty(),
                        },
                    );
                });
        },
    );

    if let Some(entry) = chosen {
        let (command, params) = action(&entry);
        match crate::menus::invoke(app, ctx, &command, params) {
            Ok(_) => {
                note_recent(app, &entry.key);
                close(app, ctx);
            }
            Err(error) => {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
    }
}

fn draw_open_badge(ui: &egui::Ui, row: egui::Rect, t: &Tokens) {
    let text = tl!("Open");
    let font = egui::FontId::proportional(10.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, t.accent_text);
    let badge = egui::Rect::from_center_size(egui::pos2(row.right() - 12.0 - galley.size().x / 2.0, row.center().y), galley.size() + vec2(12.0, 6.0));
    ui.painter().rect_filled(badge, CornerRadius::same((badge.height() / 2.0) as u8), t.accent_soft);
    ui.painter().galley(badge.center() - galley.size() / 2.0, galley, t.accent_text);
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct Metrics {
    pub card: egui::Rect,
    pub list_height: f32,
    pub rows: Vec<egui::Rect>,
    pub selected: Option<String>,
    pub query: String,
    pub empty: bool,
}

#[cfg(test)]
fn metrics_id() -> egui::Id {
    egui::Id::new("find-panel-metrics")
}

#[cfg(test)]
fn record_metrics(ctx: &egui::Context, metrics: Metrics) {
    ctx.data_mut(|d| d.insert_temp(metrics_id(), metrics));
}

/// The metrics recorded by the last drawn frame (tests and the offscreen visual QA).
#[cfg(test)]
pub(crate) fn metrics(ctx: &egui::Context) -> Option<Metrics> {
    ctx.data(|d| d.get_temp(metrics_id()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use egui::{Event, Key, Modifiers, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::{entries, metrics, ranked};
    use crate::PhotocraftApp;

    fn harness() -> Harness<'static, PhotocraftApp> {
        Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
        })
    }

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    fn open(h: &mut Harness<'static, PhotocraftApp>) {
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "window.findPanel", json!({})).expect("Find Panel… opens");
        h.run_steps(2);
    }

    fn type_query(h: &mut Harness<'static, PhotocraftApp>, query: &str) {
        h.event(Event::Text(query.into()));
        h.run_steps(2);
    }

    #[test]
    fn find_panel_is_in_the_window_menu_with_a_free_shortcut() {
        let mut app = app();
        app.run("file.new", json!({"width": 8, "height": 8})).expect("document");
        let item = crate::menus::menu_items(&app).into_iter().find(|i| i.id == "window.findPanel").expect("Window › Find Panel…");
        assert_eq!(item.path, ["Window"]);
        assert_eq!(item.shortcut.as_deref(), Some("Cmd+Shift+P"));
        assert!(item.enabled);
        // The new key belongs to the new command alone; existing commands keep theirs.
        let owners: BTreeSet<&str> = crate::menus::UI_COMMANDS
            .iter()
            .filter(|c| c.3 == Some("Cmd+Shift+P"))
            .map(|c| c.0)
            .chain(photocraft_engine::command_specs().iter().filter(|c| c.shortcut == Some("Cmd+Shift+P")).map(|c| c.id))
            .chain(crate::menu_catalog::CATALOG.iter().filter(|c| c.2 == Some("Cmd+Shift+P")).map(|c| c.3))
            .chain(photocraft_engine::prefs::ALTERNATE_SHORTCUTS.iter().filter(|c| c.1 == "Cmd+Shift+P").map(|c| c.0))
            .collect();
        assert_eq!(owners, BTreeSet::from(["window.findPanel"]), "Cmd+Shift+P is not already taken");
        assert_eq!(crate::shortcuts::default_shortcut("edit.search").as_deref(), Some("Cmd+F"), "Edit › Search keeps its key");
    }

    #[test]
    fn entries_come_from_the_dock_registry_and_the_floating_toggles() {
        let app = app();
        let list = entries(&app);
        let keys: BTreeSet<&str> = list.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys.len(), list.len(), "no duplicate rows");
        assert!(list.iter().any(|e| e.key == "dock:layers" && e.open), "the default dock's Layers is open");
        assert!(list.iter().any(|e| e.key == "dock:history" && !e.open), "History starts closed");
        assert!(list.iter().any(|e| e.key == "dock:brushes" && !e.open), "Brushes is a registered dock panel");
        assert!(list.iter().any(|e| e.key == "toggle:window.toggle.brushSettings"), "Brush Settings is a floating window");
        assert!(list.iter().any(|e| e.key == "toggle:window.toggle.retouchAI"), "Retouch AI is a floating window");
        assert!(!list.iter().any(|e| e.key == "toggle:window.toggle.brushes"), "the dock's Brushes is not listed twice");
        assert_eq!(list.iter().filter(|e| e.label == "Brushes").count(), 1, "the label is not duplicated either");
    }

    #[test]
    fn ranking_filters_by_name_and_group_and_prefers_recents() {
        let app = app();
        let list = entries(&app);
        let first = |query: &str, recents: &[String]| ranked(&list, query, recents).first().map(|i| list[*i].key.clone());
        assert_eq!(first("history", &[]).as_deref(), Some("dock:history"));
        assert_eq!(first("layers", &[]).as_deref(), Some("dock:layers"));
        assert!(first("window", &[]).is_some_and(|key| key.starts_with("toggle:")), "floating windows match by group");
        assert_eq!(first("", &["dock:history".into()]).as_deref(), Some("dock:history"), "recents lead an empty query");
        let open_first = ranked(&list, "", &[]).first().map(|i| list[*i].open);
        assert_eq!(open_first, Some(true), "open panels lead when nothing is recent");
        assert!(ranked(&list, "nothing-like-this", &[]).is_empty());
    }

    #[test]
    fn command_palette_and_find_panel_do_not_stack() {
        let mut h = harness();
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "edit.search", json!({})).unwrap();
        h.run_steps(2);
        assert!(h.state().ui.palette_open);
        crate::menus::invoke(h.state_mut(), &ctx, "window.findPanel", json!({})).unwrap();
        h.run_steps(2);
        assert!(h.state().ui.find_panel_open);
        assert!(!h.state().ui.palette_open, "only one search surface is up");
    }

    #[test]
    fn typing_filters_and_enter_opens_then_closes_a_panel() {
        let mut h = harness();
        open(&mut h);
        type_query(&mut h, "history");
        let m = metrics(&h.ctx).expect("the palette drew");
        assert_eq!(m.query, "history");
        assert_eq!(m.selected.as_deref(), Some("dock:history"), "exact name is first");
        assert!(!m.rows.is_empty(), "History matches at least its own panel");
        assert_eq!(m.selected.as_deref(), Some("dock:history"), "its own panel still ranks first");

        h.key_press(Key::Enter);
        h.run_steps(3);
        assert!(!h.state().ui.find_panel_open, "Enter closes the palette");
        assert!(crate::panel_docking::is_open(h.state(), "history"), "Enter opened History");
        assert_eq!(h.state().ui.recent_panels.first().map(String::as_str), Some("dock:history"), "the choice is remembered");

        open(&mut h);
        assert!(metrics(&h.ctx).expect("the palette drew").query.is_empty(), "reopening starts a fresh query");
        type_query(&mut h, "history");
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert!(!crate::panel_docking::is_open(h.state(), "history"), "the same row now closes History");
    }

    #[test]
    fn arrows_move_the_selection_and_enter_switches_it() {
        let mut h = harness();
        open(&mut h);
        type_query(&mut h, "history");
        let first = metrics(&h.ctx).expect("the palette drew").selected.expect("a selected row");
        h.key_press(Key::ArrowDown);
        h.run_steps(2);
        let second = metrics(&h.ctx).expect("the palette drew").selected.expect("a selected row");
        assert_ne!(first, second, "ArrowDown moves to the next match");
        let dock = second.strip_prefix("dock:").expect("a dock panel");
        let before = crate::panel_docking::is_open(h.state(), dock);
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert!(!h.state().ui.find_panel_open);
        assert_ne!(crate::panel_docking::is_open(h.state(), dock), before, "Enter switched the highlighted panel");
    }

    #[test]
    fn the_card_fits_the_window_and_an_empty_query_has_no_rows() {
        let mut h = harness();
        open(&mut h);
        let m = metrics(&h.ctx).expect("the palette drew");
        assert!(m.card.width() <= 1440.0 && m.card.height() <= 900.0, "the card stays inside 1440×900");
        assert!(m.list_height > 0.0, "the results list has room");
        assert!(!m.empty);
        type_query(&mut h, "nothing-like-this");
        let m = metrics(&h.ctx).expect("the palette drew");
        assert!(m.empty && m.rows.is_empty(), "a query with no match shows the empty state");
    }

    #[test]
    fn the_shortcut_opens_the_palette_and_escape_closes_it() {
        let mut h = harness();
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
        h.run_steps(3);
        assert!(h.state().ui.find_panel_open, "Cmd+Shift+P opens Find Panel…");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(!h.state().ui.find_panel_open, "Esc closes the palette");
    }
}

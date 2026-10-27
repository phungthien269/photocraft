//! Window › Color Presets: the floating library of whole-image colour/tone looks.
//!
//! A preset is captured from the current document's visible adjustment layers and applied as a
//! new pass-through group, in one undo step (`color.presets.new` / `.apply`; the engine owns the
//! data and the document edits). The library is global: groups, rename, delete and move work with
//! no document open. The panel also runs a folder batch through `file.automate.batch` with a
//! `color.presets.apply` step per image, into a folder that is never the source folder.
//!
//! All state lives in [`ColorPresetsState`] (part of [`crate::state::UiState`]); the browser's
//! selection, sizes and inline rename use the shared [`crate::preset_panels::PresetUi`] under the
//! panel key `color`. Every action is queued while the panel draws and applied afterwards, so no
//! UI-state borrow is held across an engine command.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, pos2, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::{DocId, Document, Layer, LayerContent};
use photocraft_engine::jobs::{JobId, Started};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::preset_panels::{self, Ev, GroupView, ItemView, Place};
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

/// The fixed card width, like Brush Settings, Retouch AI and Basic Color.
pub const WIDTH: f32 = 340.0;

/// The panel's key in the shared preset browser state.
const PANEL: &str = "color";

/// The chrome above the scrolling body (title row, hairline and the space around them).
const HEADER_HEIGHT: f32 = 43.0;

/// The body's cap: the browser scrolls and the batch stays one wheel away; the body scrolls too
/// in a short window, so nothing is pushed off-screen.
const MAX_BODY: f32 = 620.0;

/// The browser's default card size: large enough for the look hint and its layer count.
const DEFAULT_THUMB: f32 = 64.0;

/// The batch formats. PSD is deliberately absent: it cannot carry Basic Color layers.
pub(crate) const FORMATS: [&str; 4] = ["same", "jpg", "png", "tiff"];

/// The panel's id, so tests can find its rectangle in egui's memory.
fn panel_id() -> egui::Id {
    egui::Id::new("color-presets")
}

/// The panel's English source keys, as data rather than literal `tl!` calls.
///
/// Every key has a `vi.tsv` row (checked by `tests::vietnamese_covers_every_panel_key`). The
/// strings shared with other panels reuse their rows (Close, Browse…, the batch labels); the
/// other complete catalogs fall back to English for the panel-only text until a translator
/// covers it. The menu label `Color Presets` is in every complete catalog because it is a menu
/// string.
pub(crate) mod text {
    pub const TITLE: &str = "Color Presets";
    pub const CLOSE: &str = "Close";
    pub const SAVE: &str = "Save current look";
    pub const NEW: &str = "Create new color preset from the current look";
    pub const OPEN_IMAGE: &str = "Open an image to apply or save a look.";
    pub const NO_PRESETS: &str = "No presets yet. Save the current look to create one.";
    pub const SEARCH: &str = "Search presets";
    pub const NO_MATCH: &str = "No presets match your search.";
    pub const NO_LAYERS: &str = "No visible color/tone layers to save.";
    pub const APPLIED: &str = "Applied preset “{name}”.";
    pub const APPLY_FAILED: &str = "Apply preset failed: {error}";
    pub const SAVED: &str = "Saved preset “{name}”.";
    pub const SAVED_SKIPPED: &str = "Saved preset “{name}”; skipped {count} unsupported layer(s).";
    pub const SAVE_FAILED: &str = "Save preset failed: {error}";
    pub const EDIT_FAILED: &str = "Preset edit failed: {error}";

    pub const BATCH: &str = "Apply to Images";
    pub const INPUT_FOLDER: &str = "Input folder";
    pub const OUTPUT_FOLDER: &str = "Output folder";
    pub const FOLDER_PATH: &str = "Folder path";
    pub const BROWSE: &str = "Browse…";
    pub const FORMAT: &str = "Format";
    pub const SAME_AS_SOURCE: &str = "Same as source";
    pub const JPEG: &str = "JPEG";
    pub const PNG: &str = "PNG";
    pub const TIFF: &str = "TIFF";
    pub const INCLUDE_SUBFOLDERS: &str = "Include subfolders";
    pub const RUN: &str = "Run";
    pub const RUNNING: &str = "Running…";
    pub const PAUSE: &str = "Pause";
    pub const RESUME: &str = "Resume";
    pub const PAUSED: &str = "Paused at the next safe point.";
    pub const RESUMING: &str = "Resuming…";
    pub const PAUSE_HINT: &str = "Pause takes effect at the next safe point, which may be mid-file.";
    pub const SELECT_PRESET: &str = "Select a preset first.";
    pub const CHOOSE_FOLDERS: &str = "Choose both an input and an output folder.";
    pub const CHOOSE_DIFFERENT: &str = "Choose a different output folder.";
    pub const BATCH_FINISHED: &str = "Batch finished: {files} files, {errors} errors";
    pub const BATCH_FAILED: &str = "Batch failed: {error}";
    pub const BATCH_ALL_FAILED: &str = "Batch failed: every file reported an error.";
    pub const BATCH_CANCELLED: &str = "Batch cancelled. Files written by this run and the empty subfolders it created were removed.";

    pub const LAYERS: &str = "{n} layers";
    pub const ONE_LAYER: &str = "1 layer";

    /// Every fixed key above, for `tests::vietnamese_covers_every_panel_key`.
    #[cfg(test)]
    pub const ALL: &[&str] = &[
        TITLE,
        CLOSE,
        SAVE,
        NEW,
        OPEN_IMAGE,
        NO_PRESETS,
        SEARCH,
        NO_MATCH,
        NO_LAYERS,
        APPLIED,
        APPLY_FAILED,
        SAVED,
        SAVED_SKIPPED,
        SAVE_FAILED,
        EDIT_FAILED,
        BATCH,
        INPUT_FOLDER,
        OUTPUT_FOLDER,
        FOLDER_PATH,
        BROWSE,
        FORMAT,
        SAME_AS_SOURCE,
        JPEG,
        PNG,
        TIFF,
        INCLUDE_SUBFOLDERS,
        RUN,
        RUNNING,
        PAUSE,
        RESUME,
        PAUSED,
        RESUMING,
        PAUSE_HINT,
        SELECT_PRESET,
        CHOOSE_FOLDERS,
        CHOOSE_DIFFERENT,
        BATCH_FINISHED,
        BATCH_FAILED,
        BATCH_ALL_FAILED,
        BATCH_CANCELLED,
        LAYERS,
        ONE_LAYER,
    ];
}

/// Color Presets panel state, persisted in [`crate::state::UiState`] so the batch choices and the
/// last status survive a relaunch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorPresetsState {
    pub batch_input: String,
    pub batch_output: String,
    /// `same`, `jpg`, `png` or `tiff`.
    pub batch_format: String,
    pub include_subfolders: bool,
    /// The batch section is open (it starts open: the feature must be discoverable).
    pub batch_open: bool,
    /// The preset browser's name filter.
    pub search: String,
    /// The last save/apply/edit status.
    pub status: String,
    pub status_error: bool,
    /// The batch's own line, separate so a batch result doesn't hide a save/apply status.
    pub batch_status: String,
    pub batch_status_error: bool,
    /// True while the background batch job is in flight; Run is disabled meanwhile.
    #[serde(skip)]
    pub batch_running: bool,
    /// A validated batch waiting for the next frame, so the busy state paints before it starts.
    #[serde(skip)]
    pub(crate) batch_pending: Option<Value>,
    /// The running batch job. Session-only.
    #[serde(skip)]
    pub batch_job: Option<JobId>,
    /// Progress and message of the running batch (0..=1).
    #[serde(skip)]
    pub batch_progress: f32,
    #[serde(skip)]
    pub batch_message: String,
    /// The running batch is paused (it stops at its next safe point). Session-only.
    #[serde(skip)]
    pub batch_paused: bool,
}

impl Default for ColorPresetsState {
    fn default() -> Self {
        Self {
            batch_input: String::new(),
            batch_output: String::new(),
            batch_format: "same".into(),
            include_subfolders: false,
            batch_open: true,
            search: String::new(),
            status: String::new(),
            status_error: false,
            batch_status: String::new(),
            batch_status_error: false,
            batch_running: false,
            batch_pending: None,
            batch_job: None,
            batch_progress: 0.0,
            batch_message: String::new(),
            batch_paused: false,
        }
    }
}

// ------------------------------------------------------------------ library data

/// A basic-colour hint for the item card: only the four most visible parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct BasicHint {
    temperature: f32,
    tint: f32,
    exposure: f32,
    saturation: f32,
}

impl BasicHint {
    fn from_params(v: &Value) -> Self {
        let number = |k: &str| v.get(k).and_then(Value::as_f64).map(|n| n as f32).filter(|n| n.is_finite()).unwrap_or(0.0);
        Self {
            temperature: number("temperature").clamp(-100.0, 100.0),
            tint: number("tint").clamp(-100.0, 100.0),
            exposure: number("exposure").clamp(-5.0, 5.0),
            saturation: number("saturation").clamp(-100.0, 100.0),
        }
    }
}

#[derive(Clone, Debug)]
struct LibraryItem {
    name: String,
    /// The preset's layers exactly as `color.presets.list` returns them, bottom-to-top.
    layers: Vec<Value>,
    layer_count: usize,
    /// A stable hash of `layers`, so the preview cache notices an edited preset without rehashing.
    layers_hash: u64,
    hint: Option<BasicHint>,
}

/// A hash of the preset's layer JSON: the cache key's "what does this preset do" part.
fn hash_layers(layers: &[Value]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    layers.len().hash(&mut hasher);
    for layer in layers {
        layer.to_string().hash(&mut hasher);
    }
    hasher.finish()
}

#[derive(Clone, Debug)]
struct LibraryGroup {
    name: String,
    items: Vec<LibraryItem>,
}

/// The library as `color.presets.list` returns it, reduced to what the browser draws.
#[derive(Clone, Debug, Default)]
struct Library {
    groups: Vec<LibraryGroup>,
    total: usize,
}

fn parse_library(value: &Value) -> Library {
    let mut library = Library::default();
    for group in value.get("groups").and_then(Value::as_array).into_iter().flatten() {
        let Some(name) = group.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else { continue };
        let mut items = Vec::new();
        for preset in group.get("presets").and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = preset.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else { continue };
            let layers = preset.get("layers").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
            let hint = layers
                .iter()
                .find(|layer| layer.get("kind").and_then(Value::as_str) == Some("basicColor"))
                .and_then(|layer| layer.get("params"))
                .map(BasicHint::from_params);
            items.push(LibraryItem { name: name.to_string(), layers: layers.to_vec(), layer_count: layers.len(), layers_hash: hash_layers(layers), hint });
        }
        library.total += items.len();
        library.groups.push(LibraryGroup { name: name.to_string(), items });
    }
    library
}

/// The library reduced to the preset names containing `needle` (case-insensitive).
fn filter_library(library: &Library, needle: &str) -> Library {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return library.clone();
    }
    let mut out = Library::default();
    for group in &library.groups {
        let items: Vec<LibraryItem> = group.items.iter().filter(|item| item.name.to_lowercase().contains(&needle)).cloned().collect();
        if !items.is_empty() {
            out.total += items.len();
            out.groups.push(LibraryGroup { name: group.name.clone(), items });
        }
    }
    out
}

/// The longest side of a preset preview, in pixels.
const PREVIEW_SIDE: u32 = 96;

/// The most preview textures kept at once; a larger library renders the rest on demand.
const MAX_PREVIEWS: usize = 256;

fn preview_cache_id() -> egui::Id {
    egui::Id::new("color-presets-previews")
}

/// One adjustment layer of a preset, rebuilt from its stored JSON. `None` for a kind this build
/// cannot recreate; an unknown blend falls back to Normal, and opacity is clamped like `apply`.
fn preview_layer(value: &Value, mode: photocraft_doc::ColorMode) -> Option<Layer> {
    let kind = value.get("kind").and_then(Value::as_str).map(str::trim).filter(|k| !k.is_empty())?;
    let params = value.get("params").unwrap_or(&Value::Null);
    let adjustment = photocraft_engine::adjust_params::from_params(kind, params, None, mode).ok()?;
    let opacity = match value.get("opacity") {
        None | Some(Value::Null) => 1.0,
        Some(v) => (v.as_f64().filter(|o| o.is_finite())? as f32).clamp(0.0, 1.0),
    };
    let blend = value
        .get("blend")
        .and_then(Value::as_str)
        .and_then(photocraft_engine::commands::blend_from_str)
        .filter(|b| *b != BlendMode::PassThrough)
        .unwrap_or(BlendMode::Normal);
    let name = match value.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => adjustment.label().to_string(),
    };
    let mut layer = Layer::new(name, LayerContent::Adjustment(adjustment));
    layer.opacity = opacity;
    layer.blend = blend;
    Some(layer)
}

/// A shallow clone of `doc` with the preset's adjustment layers on top, bottom-to-top. The live
/// document is never touched; `None` when no layer can be recreated, so the caller keeps its hint.
fn preview_document(doc: &Document, item: &LibraryItem) -> Option<Document> {
    let mut preview = doc.clone();
    let before = preview.layers.len();
    for value in &item.layers {
        if let Some(layer) = preview_layer(value, doc.mode) {
            preview.layers.push(layer);
        }
    }
    (preview.layers.len() > before).then_some(preview)
}

/// The preset applied to a small render of the current image, at most `max_side` pixels on its
/// longer side. `None` when no preset layer could be recreated (the card keeps its look wash).
fn preview_image(doc: &Document, item: &LibraryItem, max_side: u32) -> Option<egui::ColorImage> {
    let preview = preview_document(doc, item)?;
    let b = preview.bounds();
    let longest = b.width().max(b.height()).max(1);
    let scale = (max_side.max(1) as f32 / longest as f32).min(1.0);
    let w = ((b.width() as f32 * scale).round() as u32).max(1);
    let h = ((b.height() as f32 * scale).round() as u32).max(1);
    let image = photocraft_compose::render_reduced(&preview, w, h).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.pixels))
}

/// The identity of one cached preview: the document state, the preset and its layers, and the size.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PreviewKey {
    document: DocId,
    revision: u64,
    name: String,
    layers: u64,
    side: u32,
}

/// One cached preview and the frame that last drew it, for the cache's least-recently-used trim.
#[derive(Clone)]
struct PreviewEntry {
    texture: egui::TextureHandle,
    used: u64,
}

/// Preview textures kept in egui's context data across frames. A new document or revision retires
/// every old texture; the map is capped so a huge library cannot grow it without bound.
#[derive(Clone, Default)]
struct PreviewCache {
    textures: HashMap<PreviewKey, PreviewEntry>,
    seen: Option<(DocId, u64)>,
    /// A use counter: every look-up and insertion gets its own stamp, so the trim has a strict order.
    stamp: u64,
}

impl PreviewCache {
    /// Drop every texture from another document state and trim the least recently used ones. Called
    /// once per frame, before drawing, so no texture this frame's paint list refers to is freed.
    fn begin(&mut self, current: Option<(DocId, u64)>) {
        if self.seen != current {
            self.textures.clear();
            self.seen = current;
        }
        if self.textures.len() > MAX_PREVIEWS {
            let mut used: Vec<u64> = self.textures.values().map(|entry| entry.used).collect();
            used.sort_unstable_by(|a, b| b.cmp(a));
            let cutoff = used.get(MAX_PREVIEWS - 1).copied().unwrap_or(0);
            self.textures.retain(|_, entry| entry.used >= cutoff);
        }
    }

    fn texture(&mut self, ctx: &egui::Context, doc: &Document, revision: u64, item: &LibraryItem) -> Option<egui::TextureHandle> {
        let key = PreviewKey { document: doc.id, revision, name: item.name.clone(), layers: item.layers_hash, side: PREVIEW_SIDE };
        self.stamp = self.stamp.wrapping_add(1);
        let stamp = self.stamp;
        if let Some(entry) = self.textures.get_mut(&key) {
            entry.used = stamp;
            return Some(entry.texture.clone());
        }
        let image = preview_image(doc, item, PREVIEW_SIDE)?;
        let tex = ctx.load_texture(format!("color-preset-{}-{:016x}", item.name, item.layers_hash), image, egui::TextureOptions::LINEAR);
        self.textures.insert(key, PreviewEntry { texture: tex.clone(), used: stamp });
        Some(tex)
    }
}

/// Read the library through its command, as the panel contract specifies.
fn fetch_library(app: &mut PhotocraftApp) -> Library {
    match app.run("color.presets.list", json!({})) {
        Ok(value) => parse_library(&value),
        Err(_) => Library::default(),
    }
}

/// The group holding the selected preset, so Save lands in the folder the user is looking at.
fn selected_group(app: &PhotocraftApp) -> Option<String> {
    let selected = app.ui.presets_ui.selected.get(PANEL)?;
    app.session.presets.color_presets.iter().find(|group| group.items.iter().any(|preset| preset.name == *selected)).map(|group| group.name.clone())
}

/// Whether Save would capture anything: a visible, supported adjustment layer, walking visible
/// groups (a hidden group hides its whole subtree, as the engine's capture does).
fn has_capturable_adjustment(app: &PhotocraftApp) -> bool {
    fn rec(layers: &[photocraft_doc::Layer]) -> bool {
        layers.iter().any(|layer| {
            layer.visible
                && match &layer.content {
                    photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::Unsupported { .. }) => false,
                    photocraft_doc::LayerContent::Adjustment(_) => true,
                    photocraft_doc::LayerContent::Group(group) => rec(&group.children),
                    _ => false,
                }
        })
    }
    app.session.active().is_some_and(|state| rec(&state.doc.layers))
}

// ------------------------------------------------------------------ status

fn set_status(app: &mut PhotocraftApp, message: String, error: bool) {
    app.ui.color_presets.status = message;
    app.ui.color_presets.status_error = error;
}

fn set_batch_status(app: &mut PhotocraftApp, message: String, error: bool) {
    app.ui.color_presets.batch_status = message;
    app.ui.color_presets.batch_status_error = error;
}

fn failed_edit(app: &mut PhotocraftApp, error: &str) {
    let message = crate::i18n::fmt(tl!(text::EDIT_FAILED), &[("error", error)]);
    set_status(app, message, true);
}

// ------------------------------------------------------------------ commands

/// Where a saved preset lands: open its group, select it and start the inline rename.
fn after_save(app: &mut PhotocraftApp, name: &str, group: &str, skipped: u64) {
    app.ui.presets_ui.selected.insert(PANEL.into(), name.to_string());
    app.ui.presets_ui.renaming = Some((PANEL.into(), name.to_string(), name.to_string()));
    open_group(app, group);
    let message = if skipped > 0 {
        crate::i18n::fmt(tl!(text::SAVED_SKIPPED), &[("name", name), ("count", &skipped.to_string())])
    } else {
        crate::i18n::fmt(tl!(text::SAVED), &[("name", name)])
    };
    set_status(app, message, false);
}

/// Open `group` although only the first folder starts open; a header click closes it again.
fn open_group(app: &mut PhotocraftApp, group: &str) {
    let key = format!("{PANEL}/{group}");
    app.ui.presets_ui.toggled.remove(&key);
    let first = app.session.presets.color_presets.first().is_some_and(|g| g.name == group);
    if first {
        app.ui.presets_ui.open_groups.remove(&key);
    } else {
        app.ui.presets_ui.open_groups.insert(key);
    }
}

fn save_current(app: &mut PhotocraftApp) {
    set_status(app, String::new(), false);
    if app.session.active_index().is_none() {
        set_status(app, tl!(text::OPEN_IMAGE).into(), false);
        return;
    }
    let group = selected_group(app);
    let params = group.map_or_else(|| json!({}), |group| json!({"group": group}));
    match app.run("color.presets.new", params) {
        Ok(value) => {
            let name = value.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let group = value.get("group").and_then(Value::as_str).unwrap_or_default().to_string();
            let skipped = value.get("skipped").and_then(Value::as_u64).unwrap_or(0);
            after_save(app, &name, &group, skipped);
        }
        Err(error) if error.contains("no visible adjustment layers") => {
            set_status(app, tl!(text::NO_LAYERS).into(), true);
        }
        Err(error) => {
            let message = crate::i18n::fmt(tl!(text::SAVE_FAILED), &[("error", &error)]);
            set_status(app, message, true);
        }
    }
}

fn apply_preset(app: &mut PhotocraftApp, name: &str) {
    set_status(app, String::new(), false);
    if app.session.active_index().is_none() {
        // Browsing works without a document; applying politely asks for one.
        set_status(app, tl!(text::OPEN_IMAGE).into(), false);
        return;
    }
    match app.run("color.presets.apply", json!({"preset": name})) {
        Ok(_) => {
            let message = crate::i18n::fmt(tl!(text::APPLIED), &[("name", name)]);
            set_status(app, message, false);
        }
        Err(error) => {
            let message = crate::i18n::fmt(tl!(text::APPLY_FAILED), &[("error", &error)]);
            set_status(app, message, true);
        }
    }
}

/// The batch's format dropdown: an id and its English source label.
fn format_source(id: &str) -> &'static str {
    match id {
        "jpg" => text::JPEG,
        "png" => text::PNG,
        "tiff" => text::TIFF,
        _ => text::SAME_AS_SOURCE,
    }
}

fn format_label(id: &str) -> &'static str {
    tl!(format_source(id))
}

/// Compare folder paths as typed, ignoring trailing separators, so `/photos` and `/photos/` count
/// as the same folder (the source must never be overwritten).
fn same_folder(a: &str, b: &str) -> bool {
    a.trim().trim_end_matches(['/', '\\']) == b.trim().trim_end_matches(['/', '\\'])
}

/// Validate the batch fields; a valid request waits one frame so the busy state paints before the
/// synchronous `file.automate.batch` call blocks the UI.
fn queue_batch(app: &mut PhotocraftApp) {
    set_batch_status(app, String::new(), false);
    let Some(preset) = app.ui.presets_ui.selected.get(PANEL).cloned().filter(|name| !name.is_empty()) else {
        set_batch_status(app, tl!(text::SELECT_PRESET).into(), true);
        return;
    };
    if app.session.active_index().is_none() {
        set_batch_status(app, tl!(text::OPEN_IMAGE).into(), false);
        return;
    }
    let (input, output, format, include_subfolders) = {
        let state = &app.ui.color_presets;
        (state.batch_input.trim().to_string(), state.batch_output.trim().to_string(), state.batch_format.clone(), state.include_subfolders)
    };
    if input.is_empty() || output.is_empty() {
        set_batch_status(app, tl!(text::CHOOSE_FOLDERS).into(), true);
        return;
    }
    if same_folder(&input, &output) {
        set_batch_status(app, tl!(text::CHOOSE_DIFFERENT).into(), true);
        return;
    }
    app.ui.color_presets.batch_pending = Some(json!({
        "steps": [["color.presets.apply", {"preset": preset}]],
        "input": input,
        "output": output,
        "format": format,
        "includeSubfolders": include_subfolders,
        // A cancelled run takes back the files it wrote (Color Presets owns its batch output).
        "cleanupOnCancel": true,
    }));
}

fn clear_batch_job(app: &mut PhotocraftApp) {
    app.ui.color_presets.batch_job = None;
    app.ui.color_presets.batch_running = false;
    app.ui.color_presets.batch_progress = 0.0;
    app.ui.color_presets.batch_message.clear();
    app.ui.color_presets.batch_paused = false;
}

fn finish_batch_value(app: &mut PhotocraftApp, value: &Value) {
    let (message, error) = batch_status(value);
    set_batch_status(app, message, error);
    clear_batch_job(app);
}

fn finish_batch_error(app: &mut PhotocraftApp, error: &str) {
    let message = crate::i18n::fmt(tl!(text::BATCH_FAILED), &[("error", error)]);
    set_batch_status(app, message, true);
    clear_batch_job(app);
}

/// Run a validated batch as a background job (on the frame after [`queue_batch`], once the busy
/// state is painted). The engine reports per-file progress and checks pause and cancellation
/// between files, and inside a file at its next safe point.
fn execute_batch(app: &mut PhotocraftApp, params: Value) {
    match app.session.start("file.automate.batch", params) {
        Ok(Started::Job(id)) => {
            app.ui.color_presets.batch_job = Some(id);
            app.ui.color_presets.batch_running = true;
            app.ui.color_presets.batch_progress = 0.0;
            app.ui.color_presets.batch_paused = false;
            app.ui.color_presets.batch_message = tl!(text::RUNNING).to_string();
        }
        Ok(Started::Done(value)) => finish_batch_value(app, &value),
        Err(error) => finish_batch_error(app, &error.to_string()),
    }
}

/// Pick up a finished batch job; the global jobs tick applies it before this runs.
fn poll_batch_job(app: &mut PhotocraftApp) {
    let Some(id) = app.ui.color_presets.batch_job else { return };
    let Some(info) = app.session.jobs_with_recent().into_iter().find(|j| j.id == id) else {
        clear_batch_job(app);
        return;
    };
    match info.state {
        "running" => {
            app.ui.color_presets.batch_progress = info.progress;
            app.ui.color_presets.batch_paused = info.paused;
            app.ui.color_presets.batch_message = if info.paused {
                tl!(text::PAUSED).to_string()
            } else if info.message.is_empty() {
                tl!(text::RUNNING).to_string()
            } else {
                info.message.clone()
            };
        }
        "done" => {
            if let Some(result) = info.result {
                finish_batch_value(app, &result);
            } else {
                clear_batch_job(app);
            }
        }
        "failed" => finish_batch_error(app, info.error.as_deref().unwrap_or("batch failed")),
        "cancelled" => {
            set_batch_status(app, tl!(text::BATCH_CANCELLED).to_string(), false);
            clear_batch_job(app);
        }
        _ => {}
    }
}

/// The status line for a finished batch: never a success when every file failed.
fn batch_status(value: &Value) -> (String, bool) {
    let files = value.get("files").and_then(Value::as_array).map_or(0, Vec::len);
    let errors = value.get("errors").and_then(Value::as_array).map_or(0, Vec::len);
    if files == 0 && errors > 0 {
        (tl!(text::BATCH_ALL_FAILED).into(), true)
    } else {
        let message = crate::i18n::fmt(tl!(text::BATCH_FINISHED), &[("files", &files.to_string()), ("errors", &errors.to_string())]);
        (message, errors > 0)
    }
}

// ------------------------------------------------------------------ events

/// Apply one browser gesture. A single click is Select; Activate (double-click) is ignored so the
/// same gesture can't apply twice.
fn handle_event(app: &mut PhotocraftApp, event: &Ev, library: &Library) {
    match event {
        Ev::Select(name) | Ev::Drop(name, _) => {
            app.ui.presets_ui.selected.insert(PANEL.into(), name.clone());
            apply_preset(app, name);
        }
        Ev::Activate(_) => {}
        Ev::Rename(key, name) => {
            let params = json!({"action": "rename", "preset": key, "name": name});
            match app.run("color.presets.edit", params) {
                Ok(value) => {
                    let renamed = value.get("name").and_then(Value::as_str).unwrap_or(name).to_string();
                    if app.ui.presets_ui.selected.get(PANEL) == Some(key) {
                        app.ui.presets_ui.selected.insert(PANEL.into(), renamed);
                    }
                }
                Err(error) => failed_edit(app, &error),
            }
        }
        Ev::Delete(key) => {
            let selected = app.ui.presets_ui.selected.get(PANEL) == Some(key);
            match app.run("color.presets.edit", json!({"action": "delete", "preset": key})) {
                Ok(_) => {
                    if selected {
                        app.ui.presets_ui.selected.remove(PANEL);
                    }
                }
                Err(error) => failed_edit(app, &error),
            }
        }
        Ev::Move(key, group) => {
            if let Err(error) = app.run("color.presets.edit", json!({"action": "move", "preset": key, "to": group})) {
                failed_edit(app, &error);
            }
        }
        Ev::RenameGroup(group, name) => match app.run("color.presets.edit", json!({"action": "renameGroup", "group": group, "name": name})) {
            Ok(_) => {
                let old = format!("{PANEL}/{group}");
                let new = format!("{PANEL}/{name}");
                if app.ui.presets_ui.open_groups.remove(&old) {
                    app.ui.presets_ui.open_groups.insert(new.clone());
                }
                if app.ui.presets_ui.toggled.remove(&old) {
                    app.ui.presets_ui.toggled.insert(new);
                }
                if let Some(order) = app.ui.presets_ui.group_order.get_mut(PANEL) {
                    for entry in order {
                        if entry == group {
                            *entry = name.clone();
                        }
                    }
                }
            }
            Err(error) => failed_edit(app, &error),
        },
        Ev::DeleteGroup(group) => {
            let selected = app.ui.presets_ui.selected.get(PANEL).cloned();
            let selected_here = library
                .groups
                .iter()
                .find(|g| g.name == *group)
                .is_some_and(|g| selected.as_deref().is_some_and(|name| g.items.iter().any(|item| item.name == name)));
            match app.run("color.presets.edit", json!({"action": "deleteGroup", "group": group})) {
                Ok(_) => {
                    if selected_here {
                        app.ui.presets_ui.selected.remove(PANEL);
                    }
                    let key = format!("{PANEL}/{group}");
                    app.ui.presets_ui.open_groups.remove(&key);
                    app.ui.presets_ui.toggled.remove(&key);
                    if let Some(order) = app.ui.presets_ui.group_order.get_mut(PANEL) {
                        order.retain(|entry| entry != group);
                    }
                }
                Err(error) => failed_edit(app, &error),
            }
        }
        Ev::NewGroup => match app.run("color.presets.edit", json!({"action": "newGroup", "name": "Group"})) {
            Ok(value) => {
                let group = value.get("group").and_then(Value::as_str).unwrap_or_default().to_string();
                app.ui.presets_ui.renaming = Some((PANEL.into(), format!("group:{group}"), group.clone()));
                open_group(app, &group);
            }
            Err(error) => failed_edit(app, &error),
        },
        Ev::New => save_current(app),
    }
}

/// What the panel asks the app to do after drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Save,
    PickInput,
    PickOutput,
    RunBatch,
    PauseBatch,
    ResumeBatch,
    CancelBatch,
}

fn apply_action(app: &mut PhotocraftApp, action: Action) {
    match action {
        Action::Save => save_current(app),
        Action::PickInput => {
            let _ = app.ask_folder(|app, path| {
                app.ui.color_presets.batch_input = path;
                Ok(Value::Null)
            });
        }
        Action::PickOutput => {
            let _ = app.ask_folder(|app, path| {
                app.ui.color_presets.batch_output = path;
                Ok(Value::Null)
            });
        }
        Action::RunBatch => queue_batch(app),
        Action::PauseBatch => {
            if let Some(id) = app.ui.color_presets.batch_job
                && app.session.pause_job(id)
            {
                app.ui.color_presets.batch_paused = true;
                app.ui.color_presets.batch_message = tl!(text::PAUSED).to_string();
                set_batch_status(app, tl!(text::PAUSED).to_string(), false);
            }
        }
        Action::ResumeBatch => {
            if let Some(id) = app.ui.color_presets.batch_job
                && app.session.resume_job(id)
            {
                app.ui.color_presets.batch_paused = false;
                app.ui.color_presets.batch_message = tl!(text::RESUMING).to_string();
                set_batch_status(app, tl!(text::RESUMING).to_string(), false);
            }
        }
        Action::CancelBatch => {
            if let Some(id) = app.ui.color_presets.batch_job {
                app.session.cancel_job(id);
            }
            set_batch_status(app, tl!(text::BATCH_CANCELLED).to_string(), false);
            clear_batch_job(app);
        }
    }
}

// ------------------------------------------------------------------ drawing

fn card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(10))
}

fn rgba(c: Color32) -> [f32; 4] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0, c.a() as f32 / 255.0]
}

fn neutral_stops(t: &Tokens) -> [(f32, [f32; 4]); 2] {
    [(0.0, rgba(t.field)), (1.0, rgba(t.hover))]
}

/// A shadow and a highlight shifted by the preset's Basic Color: warm/cool, tint, exposure and
/// saturation. The card is a look hint, not a rendering of the current image.
fn look_stops(hint: BasicHint) -> [(f32, [f32; 4]); 2] {
    let temp = hint.temperature / 100.0;
    let tint = hint.tint / 100.0;
    let sat = 1.0 + hint.saturation / 100.0;
    let exposure = 2f32.powf(hint.exposure * 0.3);
    let shifted = |mut c: [f32; 3]| {
        c[0] += temp * 0.18 + tint * 0.06;
        c[1] -= tint * 0.09;
        c[2] -= temp * 0.18 - tint * 0.06;
        let luma = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        for channel in &mut c {
            *channel = (luma + (*channel - luma) * sat) * exposure;
            *channel = channel.clamp(0.0, 1.0);
        }
        [c[0], c[1], c[2], 1.0]
    };
    [(0.0, shifted([0.16, 0.17, 0.19])), (1.0, shifted([0.86, 0.87, 0.84]))]
}

/// `rect` shrunk to `image`'s aspect ratio, centred, so a wide image is not stretched into a square.
fn fit_image(rect: Rect, image: egui::Vec2) -> Rect {
    if image.x <= 0.0 || image.y <= 0.0 {
        return rect;
    }
    let scale = (rect.width() / image.x).min(rect.height() / image.y);
    Rect::from_center_size(rect.center(), image * scale)
}

/// The item card: the preset rendered through the current image when that preview exists; otherwise
/// the current image thumbnail washed with the look's shadow/highlight colours, or a neutral look
/// gradient when there is no document thumbnail yet.
fn paint_item(ui: &egui::Ui, rect: Rect, item: &LibraryItem, t: &Tokens, preview: Option<&egui::TextureHandle>, thumbnail: Option<&egui::TextureHandle>) {
    let stops = item.hint.map_or_else(|| neutral_stops(t), look_stops);
    if let Some(tex) = preview {
        // A real render: fill the letterbox with the field colour and keep the image's aspect.
        ui.painter().rect_filled(rect, 0.0, t.field);
        let fitted = fit_image(rect, tex.size_vec2());
        ui.painter().image(tex.id(), fitted, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    } else if let Some(tex) = thumbnail {
        ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let wash =
            |c: [f32; 4], a: u8| Color32::from_rgba_unmultiplied((c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8, a);
        let mid = rect.center().y;
        ui.painter().rect_filled(Rect::from_min_max(rect.min, pos2(rect.right(), mid)), 0.0, wash(stops[0].1, 45));
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), mid), rect.max), 0.0, wash(stops[1].1, 45));
    } else {
        preset_panels::paint_gradient(ui, rect, &stops);
    }
    // In list mode the brush is 20 points tall; the name is drawn beside it by the browser.
    if rect.width() < 40.0 || rect.height() < 28.0 {
        return;
    }
    // The name sits on a scrim at the top, elided to the card; the layer count below.
    let name_rect = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + 13.0));
    ui.painter().rect_filled(name_rect, 0.0, t.scrim);
    let galley = crate::tab_strip::elided(ui, &item.name, FontId::proportional(8.5), t.text, rect.width() - 6.0);
    ui.painter().galley(pos2(name_rect.center().x - galley.size().x / 2.0, name_rect.center().y - galley.size().y / 2.0), galley, t.text);
    let strip_h = (rect.height() * 0.32).clamp(12.0, 18.0);
    let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - strip_h), rect.max);
    ui.painter().rect_filled(strip, 0.0, t.scrim);
    let label =
        if item.layer_count == 1 { tl!(text::ONE_LAYER).to_string() } else { crate::i18n::fmt(tl!(text::LAYERS), &[("n", &item.layer_count.to_string())]) };
    ui.painter().text(pos2(rect.center().x, strip.center().y), Align2::CENTER_CENTER, label, FontId::proportional(8.5), t.text);
}

fn folder_field(ui: &mut egui::Ui, label: &str, value: &mut String, pick: Action, actions: &mut Vec<Action>) {
    ui.label(egui::RichText::new(label).size(11.0).color(Tokens::get(ui.ctx()).text_dim));
    ui.horizontal(|ui| {
        // The Browse button's width depends on the language, so measure it rather than guessing:
        // together with the gap it is what the text field must leave free.
        let reserve = widgets::button_width(ui, &[tl!(text::BROWSE)]) + ui.spacing().item_spacing.x;
        let width = (ui.available_width() - reserve).max(90.0);
        ui.add_sized([width, 20.0], egui::TextEdit::singleline(value).hint_text(tl!(text::FOLDER_PATH)));
        if ui.button(tl!(text::BROWSE)).clicked() {
            actions.push(pick);
        }
    });
}

fn batch_section(ui: &mut egui::Ui, state: &mut ColorPresetsState, has_document: bool, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    widgets::hairline(ui);
    let open = state.batch_open;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let chevron = Rect::from_center_size(pos2(rect.left() + 7.0, rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui, chevron, if open { "chevron-down" } else { "chevron-right" }, 11.0, t.text_dim);
    ui.painter().text(pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, tl!(text::BATCH), theme::semibold(12.0), t.text);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, tl!(text::BATCH)));
    if response.clicked() {
        state.batch_open = !state.batch_open;
    }
    if !state.batch_open {
        return;
    }
    ui.add_space(3.0);
    folder_field(ui, tl!(text::INPUT_FOLDER), &mut state.batch_input, Action::PickInput, actions);
    folder_field(ui, tl!(text::OUTPUT_FOLDER), &mut state.batch_output, Action::PickOutput, actions);
    ui.horizontal(|ui| {
        ui.label(tl!(text::FORMAT));
        let mut chosen = None;
        egui::ComboBox::from_id_salt("color-presets-batch-format")
            .selected_text(format_label(&state.batch_format))
            .width(150.0)
            .icon(widgets::chevron_icon)
            .show_ui(ui, |ui| {
                for id in FORMATS {
                    if ui.selectable_label(state.batch_format == id, format_label(id)).clicked() {
                        chosen = Some(id.to_string());
                    }
                }
            });
        if let Some(id) = chosen {
            state.batch_format = id;
        }
    });
    ui.checkbox(&mut state.include_subfolders, tl!(text::INCLUDE_SUBFOLDERS));
    let run = ui.add_enabled(has_document && !state.batch_running, egui::Button::new(tl!(text::RUN)));
    if run.clicked() {
        actions.push(Action::RunBatch);
    }
    if state.batch_running {
        ui.horizontal(|ui| {
            ui.add(egui::ProgressBar::new(state.batch_progress).desired_width(140.0).show_percentage());
            let pause_label = if state.batch_paused { tl!(text::RESUME) } else { tl!(text::PAUSE) };
            let pause = widgets::secondary_button(ui, pause_label, 0.0);
            if pause.on_hover_text(tl!(text::PAUSE_HINT)).clicked() {
                actions.push(if state.batch_paused { Action::ResumeBatch } else { Action::PauseBatch });
            }
            if widgets::secondary_button(ui, tl!("Cancel"), 0.0).clicked() {
                actions.push(Action::CancelBatch);
            }
        });
        ui.add(egui::Label::new(egui::RichText::new(tl!(text::PAUSE_HINT)).size(10.5).color(t.text_faint)).wrap());
        if !state.batch_message.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new(&state.batch_message).size(11.0).color(t.text_dim)).wrap());
        }
    }
    if !state.batch_status.is_empty() {
        let color = if state.batch_status_error { t.danger } else { t.text_dim };
        ui.add(egui::Label::new(egui::RichText::new(&state.batch_status).size(11.0).color(color)).wrap());
    }
}

/// Window › Color Presets. Browsing works with no document; saving and applying ask for one.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.panels.color_presets {
        app.ui.color_presets.batch_pending = None;
        ctx.data_mut(|d| d.remove_temp::<PreviewCache>(preview_cache_id()));
        return;
    }
    poll_batch_job(app);
    // A batch validated last frame runs after this frame paints, so "Running…" is on screen.
    let running_now = app.ui.color_presets.batch_pending.take();
    if running_now.is_some() {
        app.ui.color_presets.batch_running = true;
    }
    let has_document = app.session.active_index().is_some();
    let can_save = has_capturable_adjustment(app);
    let library = fetch_library(app);
    let shown = filter_library(&library, &app.ui.color_presets.search);
    let groups: Vec<GroupView> = shown
        .groups
        .iter()
        .map(|group| GroupView {
            name: group.name.clone(),
            items: group.items.iter().map(|item| ItemView { key: item.name.clone(), name: item.name.clone() }).collect(),
        })
        .collect();
    let preview_tex = app.session.active().and_then(|st| {
        st.doc
            .layers
            .iter()
            .find(|layer| layer.visible && layer.surface().is_some())
            .and_then(|layer| app.thumbs.get(&(layer.id, crate::mask_thumbs_ui::THUMB_LAYER)))
            .map(|(_, tex)| tex.clone())
    });
    // The exact previews cache: keyed by the document state, the preset and its layers, and size.
    // Taken out of egui's data for the draw and put back after it, so no context borrow is held.
    let doc = app.session.active().map(|st| (st.doc.as_ref(), st.revision));
    let mut previews = ctx.data_mut(|d| d.remove_temp::<PreviewCache>(preview_cache_id())).unwrap_or_default();
    previews.begin(doc.map(|(doc, revision)| (doc.id, revision)));
    let mut st = std::mem::take(&mut app.ui.presets_ui);
    st.sizes.entry(PANEL.to_string()).or_insert(DEFAULT_THUMB);
    let mut events = Vec::new();
    let mut actions = Vec::new();
    let mut open = true;
    let t = Tokens::get(ctx);
    let frame = card_frame(&t);
    let canvas = crate::rulers::content_rect(app, app.last_canvas_rect);
    let frame_margin = frame.total_margin().sum().y;
    let default_top = canvas.top() + 24.0;
    let panel_top = ctx.memory(|mem| mem.area_rect(panel_id())).map_or(default_top, |rect| rect.top());
    let room = (ctx.content_rect().bottom() - panel_top.min(default_top) - frame_margin - HEADER_HEIGHT).max(0.0);
    let body_height = MAX_BODY.min(room);
    let thumb = &mut |ui: &egui::Ui, rect: Rect, gi: usize, ii: usize| {
        if let Some(item) = shown.groups.get(gi).and_then(|group| group.items.get(ii)) {
            let preview = doc.and_then(|(doc, revision)| previews.texture(ui.ctx(), doc, revision, item));
            paint_item(ui, rect, item, &t, preview.as_ref(), preview_tex.as_ref());
        }
    };
    egui::Window::new(tl!(text::TITLE))
        .id(panel_id())
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(frame)
        .default_pos(pos2((canvas.right() - WIDTH - 30.0).max(canvas.left() + 8.0), default_top))
        .min_height(HEADER_HEIGHT + body_height + frame_margin)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(tl!(text::TITLE)).font(theme::semibold(13.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 20.0, false, tl!(text::CLOSE)).clicked() {
                        open = false;
                    }
                });
            });
            ui.add_space(5.0);
            widgets::hairline(ui);
            ui.add_space(5.0);
            if !has_document {
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                ui.painter().rect_filled(rect, t.radius_sm, t.field);
                ui.painter().text(pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, tl!(text::OPEN_IMAGE), theme::medium(11.5), t.warning);
                // Painted text has no accessibility node of its own; name the banner for tests,
                // screen readers and the control channel.
                let hint = ui.interact(rect, ui.id().with("color-presets-no-document"), Sense::hover());
                hint.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tl!(text::OPEN_IMAGE)));
            }
            let body = egui::ScrollArea::vertical().id_salt("color-presets-body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let save = ui.add_enabled_ui(can_save, |ui| widgets::primary_button(ui, tl!(text::SAVE), ui.available_width())).inner;
                if save.clicked() {
                    actions.push(Action::Save);
                }
                if has_document && !can_save {
                    ui.add(egui::Label::new(egui::RichText::new(tl!(text::NO_LAYERS)).size(10.5).color(t.text_faint)).wrap());
                }
                if !app.ui.color_presets.status.is_empty() {
                    let color = if app.ui.color_presets.status_error { t.danger } else { t.text_dim };
                    ui.add(egui::Label::new(egui::RichText::new(&app.ui.color_presets.status).size(11.0).color(color)).wrap());
                }
                ui.add(egui::TextEdit::singleline(&mut app.ui.color_presets.search).hint_text(tl!(text::SEARCH)).desired_width(ui.available_width()));
                ui.add_space(3.0);
                if library.total == 0 {
                    ui.add(egui::Label::new(egui::RichText::new(tl!(text::NO_PRESETS)).size(11.0).color(t.text_faint)).wrap());
                    ui.add_space(2.0);
                } else if shown.total == 0 {
                    ui.add(egui::Label::new(egui::RichText::new(tl!(text::NO_MATCH)).size(11.0).color(t.text_faint)).wrap());
                    ui.add_space(2.0);
                }
                let max_h = (body_height - 296.0).clamp(90.0, 280.0);
                events = preset_panels::browser(ui, &mut st, PANEL, &groups, Place { canvas, max_h, new_tip: tl!(text::NEW), flush: false }, thumb);
                batch_section(ui, &mut app.ui.color_presets, has_document, &mut actions);
                ui.add_space(2.0);
            });
            // What the tests measure: the body's viewport and the height of all its content.
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(panel_id().with("body"), (body.inner_rect, body.content_size)));
            #[cfg(not(test))]
            let _ = body;
        });
    app.ui.presets_ui = st;
    ctx.data_mut(|d| d.insert_temp(preview_cache_id(), previews));
    if !open {
        app.ui.panels.color_presets = false;
    }
    for event in events {
        handle_event(app, &event, &library);
    }
    for action in actions {
        apply_action(app, action);
    }
    if let Some(params) = running_now {
        execute_batch(app, params);
        ctx.request_repaint();
    } else if app.ui.color_presets.batch_pending.is_some() || app.ui.color_presets.batch_running {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_geom::Size;

    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    fn app_with_document() -> PhotocraftApp {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).expect("new document");
        session.execute("layer.newAdjustmentLayer.basicColor", json!({})).expect("basic color layer");
        PhotocraftApp::new(session, crate::Services::default())
    }

    fn harness(mut app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
        app.ui.panels.color_presets = true;
        let mut h = Harness::builder().with_size(size).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                window(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    fn library_names(app: &PhotocraftApp) -> Vec<String> {
        app.session.presets.color_presets.iter().flat_map(|group| group.items.iter().map(|preset| preset.name.clone())).collect()
    }

    #[test]
    fn default_state_round_trips_through_serde() {
        let state = ColorPresetsState::default();
        assert_eq!(state.batch_format, "same");
        assert!(state.batch_open, "the batch stays discoverable");
        let encoded = serde_json::to_string(&state).expect("serialize");
        let back: ColorPresetsState = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(state, back);
        let fresh: ColorPresetsState = serde_json::from_str("{}").expect("missing fields default");
        assert_eq!(fresh, ColorPresetsState::default());
    }

    #[test]
    fn menu_toggle_and_panel_alias_open_the_library() {
        let mut app = app();
        let ctx = egui::Context::default();
        assert!(!app.ui.panels.color_presets);
        crate::menus::invoke(&mut app, &ctx, "window.toggle.colorPresets", json!({})).expect("toggle");
        assert!(app.ui.panels.color_presets);
        let item = crate::menus::menu_items(&app)
            .into_iter()
            .find(|item| item.label == "Color Presets" && item.path.iter().any(|p| p == "Window"))
            .expect("Window › Color Presets is in the menu");
        assert_eq!(item.checked, Some(true), "the menu shows Color Presets checked");
        crate::menus::invoke(&mut app, &ctx, "window.panel.colorPresets", json!({})).expect("panel alias");
        assert!(!app.ui.panels.color_presets);
        crate::menus::invoke(&mut app, &ctx, "window.panel.colorPresets", json!({})).expect("panel alias");
        assert!(app.ui.panels.color_presets);
    }

    #[test]
    fn menu_catalog_uses_the_toggle_command() {
        let entry = crate::menu_catalog::CATALOG
            .iter()
            .find(|(path, label, _, _)| *label == "Color Presets" && path.contains(&"Window"))
            .expect("Window › Color Presets is in the catalog");
        assert_eq!(entry.3, "window.toggle.colorPresets");
    }

    #[test]
    fn basic_color_presets_button_opens_the_library() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).expect("new document");
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.panels.basic_color = true;
        let mut h = Harness::builder().with_size(vec2(900.0, 800.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::basic_color_panel::window(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        assert!(!h.state().ui.panels.color_presets);
        h.get_by_label(crate::basic_color_panel::text::PRESETS).click();
        h.run_steps(2);
        assert!(h.state().ui.panels.color_presets, "the Basic Color button opens the library");
    }

    #[test]
    fn closing_the_panel_clears_its_flag() {
        let h_state = app_with_document();
        let mut h = harness(h_state, vec2(900.0, 800.0));
        assert!(h.state().ui.panels.color_presets);
        h.get_by_label(text::CLOSE).click();
        h.run_steps(2);
        assert!(!h.state().ui.panels.color_presets, "the close box closes the library");
    }

    #[test]
    fn save_captures_the_document_and_selects_the_new_preset() {
        let mut app = app_with_document();
        save_current(&mut app);
        assert_eq!(app.perf.last_command, "color.presets.new");
        let names = library_names(&app);
        assert!(names.contains(&"Look 1".to_string()), "the new look is saved: {names:?}");
        assert_eq!(app.ui.presets_ui.selected.get(PANEL).map(String::as_str), Some("Look 1"));
        assert!(app.ui.presets_ui.renaming.as_ref().is_some_and(|(panel, key, _)| panel == PANEL && key == "Look 1"));
        let group = selected_group(&app).expect("the saved preset has a group");
        assert!(
            app.ui.presets_ui.open_groups.contains(&format!("{PANEL}/{group}")) || app.session.presets.color_presets.first().is_some_and(|g| g.name == group)
        );
        assert!(app.ui.color_presets.status.contains("Look 1") && !app.ui.color_presets.status_error);
    }

    #[test]
    fn save_uses_the_selected_presets_group() {
        let mut app = app_with_document();
        app.ui.presets_ui.selected.insert(PANEL.into(), "Warm Portrait".into());
        save_current(&mut app);
        let group =
            app.session.presets.color_presets.iter().find(|group| group.items.iter().any(|preset| preset.name == "Warm Portrait")).expect("built-in group");
        assert!(group.items.iter().any(|preset| preset.name == "Look 1"), "Look 1 lands in the selected group");
    }

    #[test]
    fn save_without_a_document_or_layers_shows_a_clear_message() {
        let mut empty = app();
        save_current(&mut empty);
        assert_eq!(empty.ui.color_presets.status, text::OPEN_IMAGE);
        assert!(!empty.ui.color_presets.status_error);

        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 32, "height": 32, "mode": "rgb", "depth": 8})).expect("new document");
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        save_current(&mut app);
        assert_eq!(app.ui.color_presets.status, text::NO_LAYERS);
        assert!(app.ui.color_presets.status_error);
        assert!(library_names(&app).iter().all(|name| name != "Look 1"), "no empty preset was created");
    }

    #[test]
    fn applying_a_preset_calls_apply_and_reports_the_name() {
        let mut app = app_with_document();
        apply_preset(&mut app, "Warm Portrait");
        assert_eq!(app.perf.last_command, "color.presets.apply");
        assert!(app.ui.color_presets.status.contains("Warm Portrait"));
        assert!(!app.ui.color_presets.status_error);
        let st = app.session.active().expect("document");
        assert!(st.doc.layers.iter().any(|layer| layer.name == "Preset • Warm Portrait"));
    }

    #[test]
    fn applying_without_a_document_is_polite_and_does_not_call_apply() {
        let mut app = app();
        apply_preset(&mut app, "Warm Portrait");
        assert_eq!(app.ui.color_presets.status, text::OPEN_IMAGE);
        assert_ne!(app.perf.last_command, "color.presets.apply");
    }

    #[test]
    fn a_click_applies_once_when_activate_follows_select() {
        let mut app = app_with_document();
        app.ui.presets_ui.selected.insert(PANEL.into(), "Warm Portrait".into());
        let library = parse_library(&json!({"groups": [{"name": "PhotoCraft", "presets": [{"name": "Warm Portrait", "layers": []}]}]}));
        handle_event(&mut app, &Ev::Select("Warm Portrait".into()), &library);
        handle_event(&mut app, &Ev::Activate("Warm Portrait".into()), &library);
        let st = app.session.active().expect("document");
        let applied = st.doc.layers.iter().filter(|layer| layer.name == "Preset • Warm Portrait").count();
        assert_eq!(applied, 1, "Select applies, Activate is ignored");
    }

    #[test]
    fn delete_and_rename_update_the_selection() {
        let mut app = app_with_document();
        app.ui.presets_ui.selected.insert(PANEL.into(), "Warm Portrait".into());
        let library = parse_library(&json!({"groups": [{"name": "PhotoCraft", "presets": [{"name": "Warm Portrait", "layers": []}]}]}));
        handle_event(&mut app, &Ev::Rename("Warm Portrait".into(), "Warm 2".into()), &library);
        assert_eq!(app.ui.presets_ui.selected.get(PANEL).map(String::as_str), Some("Warm 2"));
        handle_event(&mut app, &Ev::Delete("Warm 2".into()), &library);
        assert!(!app.ui.presets_ui.selected.contains_key(PANEL), "the deleted selection is cleared");
        assert!(library_names(&app).iter().all(|name| name != "Warm 2"));
    }

    #[test]
    fn new_group_enters_rename_mode() {
        let mut app = app();
        let library = Library::default();
        handle_event(&mut app, &Ev::NewGroup, &library);
        let (panel, key, text) = app.ui.presets_ui.renaming.clone().expect("the new group starts renaming");
        assert_eq!(panel, PANEL);
        assert!(key.starts_with("group:"));
        assert_eq!(key.strip_prefix("group:"), Some(text.as_str()));
    }

    #[test]
    fn batch_validation_runs_before_any_command() {
        let mut app = app_with_document();
        app.ui.presets_ui.selected.remove(PANEL);
        queue_batch(&mut app);
        assert_eq!(app.ui.color_presets.batch_status, text::SELECT_PRESET);
        assert_ne!(app.perf.last_command, "file.automate.batch");

        app.ui.presets_ui.selected.insert(PANEL.into(), "Warm Portrait".into());
        app.ui.color_presets.batch_input = "/photos/in".into();
        app.ui.color_presets.batch_output.clear();
        queue_batch(&mut app);
        assert_eq!(app.ui.color_presets.batch_status, text::CHOOSE_FOLDERS);

        app.ui.color_presets.batch_output = "/photos/in/".into();
        queue_batch(&mut app);
        assert_eq!(app.ui.color_presets.batch_status, text::CHOOSE_DIFFERENT);
        assert_ne!(app.perf.last_command, "file.automate.batch");
        assert!(app.ui.color_presets.batch_status_error);
    }

    #[test]
    fn search_filters_presets_by_name_and_keeps_groups() {
        let library = parse_library(&json!({"groups": [
            {"name": "A", "presets": [{"name": "Warm", "layers": []}, {"name": "Cool", "layers": []}]},
            {"name": "B", "presets": [{"name": "Warm B", "layers": []}]}
        ]}));
        let warm = filter_library(&library, "warm");
        assert_eq!(warm.total, 2);
        assert_eq!(warm.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(warm.groups[0].items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["Warm"]);
        assert_eq!(filter_library(&library, "zzz").total, 0);
        assert_eq!(filter_library(&library, "  ").total, 3);
    }

    #[test]
    fn batch_formats_never_offer_psd() {
        assert_eq!(FORMATS, ["same", "jpg", "png", "tiff"]);
        assert!(FORMATS.iter().all(|id| !id.eq_ignore_ascii_case("psd")));
    }

    #[test]
    fn a_valid_batch_starts_one_job_and_can_be_cancelled() {
        use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
        let root = std::env::temp_dir().join(format!("photocraft-ui-batch-{}", std::process::id()));
        let input = root.join("in");
        let output = root.join("out");
        std::fs::create_dir_all(&input).expect("input folder");
        let img = Image::from_raw(1, 1, ChannelLayout::Rgb, SampleType::U8, vec![255, 0, 0]).expect("image");
        let png = photocraft_codecs::encode(&img, Format::Png, &EncodeOptions::default()).expect("png");
        std::fs::write(input.join("one.png"), png).expect("input image");
        let mut app = app_with_document();
        app.ui.presets_ui.selected.insert(PANEL.into(), "Warm Portrait".into());
        app.ui.color_presets.batch_input = input.to_string_lossy().into_owned();
        app.ui.color_presets.batch_output = output.to_string_lossy().into_owned();
        queue_batch(&mut app);
        assert_ne!(app.perf.last_command, "file.automate.batch", "validation only; the run waits a frame");
        assert!(!app.ui.color_presets.batch_running);
        let params = app.ui.color_presets.batch_pending.clone().expect("the batch is queued");
        assert_eq!(params["cleanupOnCancel"], json!(true), "a cancelled Color Presets run takes back its files");
        app.ui.color_presets.batch_pending = None;
        app.ui.color_presets.batch_running = true;
        execute_batch(&mut app, params);
        let job = app.ui.color_presets.batch_job.expect("the batch runs as a background job");
        assert!(app.ui.color_presets.batch_running);
        apply_action(&mut app, Action::CancelBatch);
        assert!(!app.ui.color_presets.batch_running && app.ui.color_presets.batch_job.is_none());
        assert_eq!(app.ui.color_presets.batch_status, text::BATCH_CANCELLED);
        assert!(app.session.job(job).is_none(), "cancel removes the running job");
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A background batch job that runs until it is cancelled, so pause/resume can be observed.
    fn gated_batch_job(app: &mut PhotocraftApp) -> JobId {
        let started = app
            .session
            .start_job(
                "file.automate.batch",
                json!({}),
                "Batch",
                false,
                |ctx| {
                    while !ctx.cancelled() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    ctx.check()
                },
                |_s, ()| Ok(json!({"files": [], "errors": []})),
            )
            .expect("start the batch job");
        match started {
            Started::Job(id) => id,
            Started::Done(_) => panic!("expected a background job"),
        }
    }

    #[test]
    fn pause_and_resume_actions_update_the_batch_state() {
        let mut app = app_with_document();
        let id = gated_batch_job(&mut app);
        app.ui.color_presets.batch_job = Some(id);
        app.ui.color_presets.batch_running = true;
        app.ui.color_presets.batch_message = text::RUNNING.to_string();
        apply_action(&mut app, Action::PauseBatch);
        assert!(app.ui.color_presets.batch_paused);
        assert!(app.session.job(id).expect("the job runs").paused);
        assert_eq!(app.ui.color_presets.batch_status, text::PAUSED);
        assert_eq!(app.ui.color_presets.batch_message, text::PAUSED);
        poll_batch_job(&mut app);
        assert!(app.ui.color_presets.batch_paused, "polling keeps the paused state");
        assert_eq!(app.ui.color_presets.batch_message, text::PAUSED);
        apply_action(&mut app, Action::ResumeBatch);
        assert!(!app.ui.color_presets.batch_paused);
        assert!(!app.session.job(id).expect("the job runs").paused);
        assert_eq!(app.ui.color_presets.batch_status, text::RESUMING);
        assert_eq!(app.ui.color_presets.batch_message, text::RESUMING);
        app.session.cancel_job(id);
        app.session.join_cancelled_jobs();
    }

    #[test]
    fn cancelled_batch_status_says_the_run_removed_its_files_and_empty_folders() {
        assert!(text::BATCH_CANCELLED.contains("removed"), "the status names the cleanup: {}", text::BATCH_CANCELLED);
        assert!(text::BATCH_CANCELLED.contains("empty subfolders"), "the status names the folder cleanup: {}", text::BATCH_CANCELLED);
        let mut app = app_with_document();
        let id = gated_batch_job(&mut app);
        app.ui.color_presets.batch_job = Some(id);
        app.ui.color_presets.batch_running = true;
        app.ui.color_presets.batch_paused = true;
        apply_action(&mut app, Action::CancelBatch);
        assert_eq!(app.ui.color_presets.batch_status, text::BATCH_CANCELLED);
        assert!(!app.ui.color_presets.batch_running);
        assert!(!app.ui.color_presets.batch_paused, "cancel clears the pause");
        assert!(app.ui.color_presets.batch_job.is_none());
        app.session.join_cancelled_jobs();
    }

    #[test]
    fn batch_status_never_claims_success_when_every_file_failed() {
        let (message, error) = batch_status(&json!({"files": [{"path": "a.png"}], "errors": []}));
        assert_eq!(message, "Batch finished: 1 files, 0 errors");
        assert!(!error);
        let (message, error) = batch_status(&json!({"files": [], "errors": [{"path": "a.png", "error": "bad"}]}));
        assert_eq!(message, text::BATCH_ALL_FAILED);
        assert!(error);
        let (message, error) = batch_status(&json!({"files": [{"path": "a.png"}], "errors": [{"path": "b.png", "error": "bad"}]}));
        assert_eq!(message, "Batch finished: 1 files, 1 errors");
        assert!(error, "partial failures are errors in the status line");
    }

    #[test]
    fn panel_draws_without_a_document() {
        let h = harness(app(), vec2(900.0, 800.0));
        assert!(h.state().ui.panels.color_presets);
        assert!(h.query_all_by_label(text::OPEN_IMAGE).next().is_some(), "the no-document hint is shown");
        assert!(h.get_by_label(text::SAVE).accesskit_node().is_disabled(), "Save needs a document");
        assert!(!h.get_by_label("Warm Portrait").accesskit_node().is_disabled(), "browsing still works");
    }

    #[test]
    fn save_is_disabled_until_a_visible_adjustment_layer_exists() {
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width": 64, "height": 64, "mode": "rgb", "depth": 8})).expect("new document");
        let mut app = PhotocraftApp::new(session, crate::Services::default());
        app.ui.panels.color_presets = true;
        let mut h = harness(app, vec2(900.0, 800.0));
        assert!(h.get_by_label(text::SAVE).accesskit_node().is_disabled(), "a plain document has nothing to save");
        assert!(h.query_all_by_label(text::NO_LAYERS).next().is_some(), "the reason is shown");
        h.state_mut().session.execute("layer.newAdjustmentLayer.basicColor", json!({})).expect("basic color layer");
        h.run_steps(2);
        assert!(!h.get_by_label(text::SAVE).accesskit_node().is_disabled(), "the adjustment layer makes Save usable");
    }

    #[test]
    fn panel_draws_with_a_document_and_stays_inside_1440_by_900() {
        let mut h = harness(app_with_document(), vec2(1440.0, 900.0));
        let rect = h.ctx.memory(|mem| mem.area_rect(panel_id())).expect("the panel is open");
        assert!(rect.bottom() <= 900.0, "the panel stays inside the window: {rect:?}");
        assert!(rect.width() <= WIDTH + 40.0, "the card keeps its fixed width: {rect:?}");
        assert!(h.query_all_by_label(text::SAVE).next().is_some());
        assert!(h.query_all_by_label(text::BATCH).next().is_some());
        // A click on a built-in applies it once.
        h.get_by_label("Warm Portrait").click();
        h.run_steps(2);
        let st = h.state().session.active().expect("document");
        assert_eq!(st.doc.layers.iter().filter(|layer| layer.name == "Preset • Warm Portrait").count(), 1);
    }

    /// The panel's data keys skip the literal scanner (the other complete catalogs fall back to
    /// English), so this test is what guarantees the Vietnamese catalog covers every string.
    #[test]
    fn vietnamese_covers_every_panel_key() {
        let vi = crate::i18n::Lang::from_code("vi").expect("vi is registered");
        let missing: Vec<_> = text::ALL.iter().copied().filter(|key| !crate::i18n::has(vi, key)).collect();
        assert!(missing.is_empty(), "vi: missing Color Presets panel strings: {missing:#?}");
        let (title, save, close) = crate::i18n::with_language(vi, || (tl!(text::TITLE), tl!(text::SAVE), tl!(text::CLOSE)));
        assert_eq!(title, "Preset màu");
        assert_eq!(save, "Lưu look hiện tại");
        assert_eq!(close, "Đóng");
    }

    fn synthetic_doc() -> Document {
        Document::with_background("preview", Size::new(64, 48), ColorMode::Rgb, SampleType::U8, Color::rgba(0.42, 0.5, 0.58, 1.0))
    }

    /// The first item of a library holding one preset with `layers`.
    fn library_item(layers: Value) -> LibraryItem {
        let mut library = parse_library(&json!({"groups": [{"name": "Test", "presets": [{"name": "Look", "layers": layers}]}]}));
        library.groups.remove(0).items.remove(0)
    }

    #[test]
    fn preview_renders_the_preset_through_the_current_image() {
        let doc = synthetic_doc();
        let item = library_item(json!([{
            "kind": "basicColor",
            "name": "Basic Color",
            "params": {"temperature": 70.0, "tint": -20.0, "exposure": 1.2, "contrast": 40.0, "saturation": 35.0},
            "opacity": 0.5,
            "blend": "Multiply"
        }]));
        let built = preview_document(&doc, &item).expect("the preset builds a preview document");
        assert_eq!(built.layers[1].opacity, 0.5, "the layer's opacity is kept");
        assert_eq!(built.layers[1].blend, BlendMode::Multiply, "the layer's blend mode is kept");
        let preview = preview_image(&doc, &item, PREVIEW_SIDE).expect("a preset preview");
        assert_eq!(preview.size, [64, 48], "the preview keeps the document's shape");
        let plain = photocraft_compose::render_reduced(&doc, 64, 48).to_rgba8();
        let plain = egui::ColorImage::from_rgba_unmultiplied([plain.width as usize, plain.height as usize], &plain.pixels);
        assert_ne!(preview.pixels, plain.pixels, "the preset changes the rendered pixels");
        assert_eq!(doc.layers.len(), 1, "the live document is untouched");
    }

    #[test]
    fn unsupported_layers_fall_back_without_panicking() {
        let doc = synthetic_doc();
        for layers in [
            json!([]),
            json!([{"kind": "notAKind", "params": {}}]),
            json!([{"kind": "basicColor", "params": "not an object"}]),
            json!([{"kind": "", "params": {}}]),
            json!([{"kind": "invert", "params": {}, "opacity": "half"}]),
        ] {
            let item = library_item(layers);
            assert!(preview_image(&doc, &item, PREVIEW_SIDE).is_none(), "nothing can be recreated, so the card keeps its wash");
        }
        // One bad layer does not hide a good one, and an unknown blend becomes Normal.
        let item = library_item(json!([
            {"kind": "notAKind", "params": {}},
            {"kind": "invert", "params": {}, "opacity": 1.0, "blend": "not a blend mode"}
        ]));
        let preview = preview_document(&doc, &item).expect("the valid layer previews");
        assert_eq!(preview.layers.len(), 2, "only the valid layer was added");
        assert_eq!(preview.layers[1].blend, BlendMode::Normal);
        assert!(preview_image(&doc, &item, PREVIEW_SIDE).is_some());
    }

    #[test]
    fn preview_textures_are_reused_until_the_document_state_changes() {
        let ctx = egui::Context::default();
        let doc = synthetic_doc();
        let item = library_item(json!([{"kind": "invert", "params": {}}]));
        let mut cache = PreviewCache::default();
        cache.begin(Some((doc.id, 1)));
        let first = cache.texture(&ctx, &doc, 1, &item).expect("the first preview");
        let again = cache.texture(&ctx, &doc, 1, &item).expect("the cached preview");
        assert_eq!(first.id(), again.id(), "the same key reuses the texture");
        cache.begin(Some((doc.id, 2)));
        let newer = cache.texture(&ctx, &doc, 2, &item).expect("the new revision's preview");
        assert_ne!(first.id(), newer.id(), "a new revision renders a new preview");
    }
}

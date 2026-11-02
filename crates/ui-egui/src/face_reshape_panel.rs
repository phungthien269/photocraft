//! Window › Face Reshape: the floating panel that drives `retouchAI.faceReshape`.
//!
//! The engine owns the 68 sliders and the non-destructive warp. This panel only edits their values:
//! six accordion groups in the engine's order, one row per slider (a label, an integer slider and
//! an integer field), a preview call after each change, Apply for the final layer, and Compare held
//! down to show the source. Every action is an engine command queued while the panel draws, so no
//! UI-state borrow is held across a command and the source layer stays active.
//!
//! Labels are a local English/Vietnamese table: the panel's 68 slider names do not belong in every
//! language catalog. Engine keys are never translated.

use egui::{Align2, CornerRadius, Sense, Stroke, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

/// The engine command this panel drives.
pub const CMD: &str = "retouchAI.faceReshape";
/// Panel width in points.
pub const WIDTH: f32 = 430.0;
/// The chrome above the scrolling body (title row, hairline and their spacing).
const HEADER_HEIGHT: f32 = 43.0;
/// The body height when there is room for all 17 sliders of the widest group.
pub const BODY_HEIGHT: f32 = 640.0;
/// The label column of a slider row.
const LABEL_WIDTH: f32 = 150.0;
/// The numeric field of a slider row.
const FIELD_WIDTH: f32 = 56.0;

fn panel_id() -> egui::Id {
    egui::Id::new("face-reshape")
}

fn card_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(10))
}

fn vietnamese() -> bool {
    crate::i18n::current().code() == "vi"
}

fn pick(en: &'static str, vi: &'static str) -> &'static str {
    if vietnamese() { vi } else { en }
}

pub const ENTRY_EN: &str = "Face Reshape";
pub const ENTRY_VI: &str = "Định hình khuôn mặt";

/// The Retouch panel's entry button and this window's title.
pub fn entry_label() -> &'static str {
    pick(ENTRY_EN, ENTRY_VI)
}

fn close_label() -> &'static str {
    pick("Close", "Đóng")
}

fn open_hint() -> &'static str {
    pick("Open an image to start", "Mở ảnh để bắt đầu")
}

fn reset_all_label() -> &'static str {
    pick("Reset All", "Đặt lại tất cả")
}

fn reset_group_label() -> &'static str {
    pick("Reset Group", "Đặt lại nhóm")
}

fn compare_label() -> &'static str {
    pick("Compare", "So sánh")
}

fn compare_hint() -> &'static str {
    pick("Hold to view the original", "Giữ để xem ảnh gốc")
}

fn apply_label() -> &'static str {
    pick("Apply", "Áp dụng")
}

fn apply_hint() -> &'static str {
    pick("Commit the preview as a layer", "Áp dụng bản xem trước thành một lớp")
}

fn busy_label() -> &'static str {
    pick("Working…", "Đang xử lý…")
}

fn preview_updated() -> &'static str {
    pick("Preview updated ({points} points).", "Đã cập nhật bản xem trước ({points} điểm).")
}

fn preview_cleared() -> &'static str {
    pick("Preview cleared.", "Đã xoá bản xem trước.")
}

fn applied() -> &'static str {
    pick("Face Reshape applied.", "Đã áp dụng định hình khuôn mặt.")
}

fn reset_done() -> &'static str {
    pick("Reset.", "Đã đặt lại.")
}

fn preview_failed() -> &'static str {
    pick("Preview failed: {error}", "Không thể xem trước: {error}")
}

fn apply_failed() -> &'static str {
    pick("Apply failed: {error}", "Không thể áp dụng: {error}")
}

fn compare_failed() -> &'static str {
    pick("Compare failed: {error}", "Không thể so sánh: {error}")
}

fn reset_failed() -> &'static str {
    pick("Reset failed: {error}", "Không thể đặt lại: {error}")
}

// ---------------------------------------------------------------------------------------------
// Groups and labels
// ---------------------------------------------------------------------------------------------

/// The six engine groups, in the engine's key order (17 + 13 + 7 + 11 + 12 + 8).
pub const GROUPS: [&[&str]; 6] = [
    &[
        "head_scale",
        "cranium_height",
        "hair_volume",
        "hairline_shift",
        "face_slim",
        "face_width",
        "face_length",
        "jaw_width",
        "jaw_angle",
        "cheekbone_shrink",
        "cheek_slim",
        "temple_fill",
        "chin_length",
        "chin_width",
        "chin_protrusion",
        "chin_cleft",
        "double_chin_warp",
    ],
    &[
        "eye_size",
        "eye_width",
        "eye_height",
        "eye_distance",
        "eye_tilt",
        "eye_position_y",
        "inner_canthus",
        "outer_canthus",
        "lower_eyelid_down",
        "pupil_scale",
        "aegyo_sal",
        "eye_brow_distance",
        "eyelid_fold_width",
    ],
    &["eyebrow_height", "eyebrow_distance", "eyebrow_length", "eyebrow_thickness", "eyebrow_arch", "eyebrow_tail", "eyebrow_tilt"],
    &[
        "nose_scale",
        "nose_wing_width",
        "nose_tip_size",
        "nose_tip_angle",
        "nose_length",
        "bridge_width",
        "bridge_curvature",
        "nasal_hump_reduction",
        "columella_height",
        "radix_height",
        "nostril_size",
    ],
    &[
        "mouth_scale",
        "mouth_width",
        "mouth_position_y",
        "philtrum_length",
        "upper_lip_thickness",
        "lower_lip_thickness",
        "smile_corners",
        "m_lip_shape",
        "lip_plumpness",
        "lip_protrusion",
        "lower_lip_to_chin",
        "teeth_align",
    ],
    &["three_section_ratio", "face_symmetry", "eye_symmetry", "eyebrow_symmetry", "neck_slim", "neck_length", "e_line_adjustment", "midface_projection"],
];

const GROUP_NAMES: [(&str, &str); 6] = [
    ("Face & Head Shape", "Khung mặt & Đầu"),
    ("Eyes & Iris", "Mắt & Tròng mắt"),
    ("Eyebrows", "Lông mày"),
    ("Nose", "Mũi"),
    ("Mouth & Lips", "Miệng & Môi"),
    ("Proportions & Neck", "Tỷ lệ & Cổ"),
];

/// Every slider's English and Vietnamese label, keyed by the unchanged engine key.
const LABELS: [(&str, &str, &str); 68] = [
    // Face and head (17).
    ("head_scale", "Head scale", "Tỷ lệ đầu"),
    ("cranium_height", "Cranium height", "Chiều cao sọ"),
    ("hair_volume", "Hair volume", "Độ dày tóc"),
    ("hairline_shift", "Hairline shift", "Dịch đường chân tóc"),
    ("face_slim", "Face slim", "Thu gọn khuôn mặt"),
    ("face_width", "Face width", "Chiều rộng khuôn mặt"),
    ("face_length", "Face length", "Chiều dài khuôn mặt"),
    ("jaw_width", "Jaw width", "Chiều rộng hàm"),
    ("jaw_angle", "Jaw angle", "Góc hàm"),
    ("cheekbone_shrink", "Cheekbone shrink", "Thu nhỏ gò má"),
    ("cheek_slim", "Cheek slim", "Gọn má"),
    ("temple_fill", "Temple fill", "Làm đầy thái dương"),
    ("chin_length", "Chin length", "Chiều dài cằm"),
    ("chin_width", "Chin width", "Chiều rộng cằm"),
    ("chin_protrusion", "Chin protrusion", "Độ nhô cằm"),
    ("chin_cleft", "Chin cleft", "Rãnh cằm"),
    ("double_chin_warp", "Double chin", "Nọng cằm"),
    // Eyes (13).
    ("eye_size", "Eye size", "Kích thước mắt"),
    ("eye_width", "Eye width", "Chiều rộng mắt"),
    ("eye_height", "Eye height", "Chiều cao mắt"),
    ("eye_distance", "Eye distance", "Khoảng cách hai mắt"),
    ("eye_tilt", "Eye tilt", "Độ nghiêng mắt"),
    ("eye_position_y", "Eye vertical position", "Vị trí mắt (dọc)"),
    ("inner_canthus", "Inner canthus", "Đuôi mắt trong"),
    ("outer_canthus", "Outer canthus", "Đuôi mắt ngoài"),
    ("lower_eyelid_down", "Lower eyelid down", "Hạ mí dưới"),
    ("pupil_scale", "Pupil scale", "Tỷ lệ đồng tử"),
    ("aegyo_sal", "Aegyo sal", "Bọng mắt cười"),
    ("eye_brow_distance", "Eye–brow distance", "Khoảng cách mắt – lông mày"),
    ("eyelid_fold_width", "Eyelid fold width", "Độ rộng nếp mí"),
    // Eyebrows (7).
    ("eyebrow_height", "Eyebrow height", "Chiều cao lông mày"),
    ("eyebrow_distance", "Eyebrow distance", "Khoảng cách lông mày"),
    ("eyebrow_length", "Eyebrow length", "Chiều dài lông mày"),
    ("eyebrow_thickness", "Eyebrow thickness", "Độ dày lông mày"),
    ("eyebrow_arch", "Eyebrow arch", "Độ cong lông mày"),
    ("eyebrow_tail", "Eyebrow tail", "Đuôi lông mày"),
    ("eyebrow_tilt", "Eyebrow tilt", "Độ nghiêng lông mày"),
    // Nose (11).
    ("nose_scale", "Nose scale", "Tỷ lệ mũi"),
    ("nose_wing_width", "Nose wing width", "Chiều rộng cánh mũi"),
    ("nose_tip_size", "Nose tip size", "Kích thước đầu mũi"),
    ("nose_tip_angle", "Nose tip angle", "Góc đầu mũi"),
    ("nose_length", "Nose length", "Chiều dài mũi"),
    ("bridge_width", "Bridge width", "Chiều rộng sống mũi"),
    ("bridge_curvature", "Bridge curvature", "Độ cong sống mũi"),
    ("nasal_hump_reduction", "Nasal hump reduction", "Giảm gồ sống mũi"),
    ("columella_height", "Columella height", "Chiều cao trụ mũi"),
    ("radix_height", "Radix height", "Chiều cao gốc mũi"),
    ("nostril_size", "Nostril size", "Kích thước lỗ mũi"),
    // Mouth (12).
    ("mouth_scale", "Mouth scale", "Tỷ lệ miệng"),
    ("mouth_width", "Mouth width", "Chiều rộng miệng"),
    ("mouth_position_y", "Mouth vertical position", "Vị trí miệng (dọc)"),
    ("philtrum_length", "Philtrum length", "Chiều dài nhân trung"),
    ("upper_lip_thickness", "Upper lip thickness", "Độ dày môi trên"),
    ("lower_lip_thickness", "Lower lip thickness", "Độ dày môi dưới"),
    ("smile_corners", "Smile corners", "Khoé miệng cười"),
    ("m_lip_shape", "M lip shape", "Dáng môi chữ M"),
    ("lip_plumpness", "Lip plumpness", "Độ căng mọng môi"),
    ("lip_protrusion", "Lip protrusion", "Độ nhô môi"),
    ("lower_lip_to_chin", "Lower lip–chin distance", "Khoảng cách môi dưới – cằm"),
    ("teeth_align", "Teeth align", "Căn chỉnh răng"),
    // Proportions, symmetry, neck and depth (8).
    ("three_section_ratio", "Three-section ratio", "Tỷ lệ ba phần mặt"),
    ("face_symmetry", "Face symmetry", "Đối xứng khuôn mặt"),
    ("eye_symmetry", "Eye symmetry", "Đối xứng mắt"),
    ("eyebrow_symmetry", "Eyebrow symmetry", "Đối xứng lông mày"),
    ("neck_slim", "Neck slim", "Thu gọn cổ"),
    ("neck_length", "Neck length", "Chiều dài cổ"),
    ("e_line_adjustment", "E-line adjustment", "Điều chỉnh đường E"),
    ("midface_projection", "Midface projection", "Độ nhô vùng giữa mặt"),
];

/// A group's title in the active UI language.
pub fn group_title(index: usize) -> &'static str {
    let pair = GROUP_NAMES.get(index).copied().unwrap_or(GROUP_NAMES[0]);
    pick(pair.0, pair.1)
}

/// A slider's label in the active UI language (the key if the table is ever missing one).
pub fn label(key: &'static str) -> &'static str {
    LABELS.iter().find(|(k, _, _)| *k == key).map_or(key, |(_, en, vi)| pick(en, vi))
}

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

macro_rules! face_reshape_params {
    ($($field:ident),* $(,)?) => {
        /// The 68 engine sliders, in the engine's order. Values are integers in `-100..=100`.
        #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct FaceReshapeParams {
            $(pub $field: f32,)*
        }

        impl FaceReshapeParams {
            /// Every slider key, in the engine's documented order.
            pub const KEYS: &'static [&'static str] = &[$(stringify!($field)),*];
            /// Number of sliders (68).
            pub const COUNT: usize = Self::KEYS.len();

            /// A slider by key.
            pub fn get(&self, key: &str) -> Option<f32> {
                match key {
                    $(stringify!($field) => Some(self.$field),)*
                    _ => None,
                }
            }

            /// A slider by key, for editing.
            pub fn get_mut(&mut self, key: &str) -> Option<&mut f32> {
                match key {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }

            /// Set one slider; unknown keys are ignored.
            pub fn set(&mut self, key: &str, value: f32) {
                if let Some(slot) = self.get_mut(key) {
                    *slot = if value.is_finite() { value.clamp(-100.0, 100.0).round() } else { 0.0 };
                }
            }

            /// `(key, value)` for every slider, in [`Self::KEYS`] order.
            pub fn entries(&self) -> [(&'static str, f32); Self::COUNT] {
                [$( (stringify!($field), self.$field) ),*]
            }

            /// True when every slider is at 0, so the warp is the identity.
            pub fn is_identity(&self) -> bool {
                $(self.$field == 0.0)&&*
            }

            /// Zero every slider in one group.
            pub fn zero_group(&mut self, group: usize) {
                if let Some(keys) = GROUPS.get(group) {
                    for key in *keys {
                        if let Some(slot) = self.get_mut(key) {
                            *slot = 0.0;
                        }
                    }
                }
            }

            /// The full `params` object the engine expects.
            pub fn to_json(&self) -> Value {
                let mut object = serde_json::Map::with_capacity(Self::COUNT);
                for (key, value) in self.entries() {
                    object.insert(key.to_string(), json!(value));
                }
                Value::Object(object)
            }
        }
    };
}

face_reshape_params! {
    // Head and global shape.
    head_scale,
    cranium_height,
    hair_volume,
    hairline_shift,
    face_slim,
    face_width,
    face_length,
    jaw_width,
    jaw_angle,
    cheekbone_shrink,
    cheek_slim,
    temple_fill,
    chin_length,
    chin_width,
    chin_protrusion,
    chin_cleft,
    double_chin_warp,
    // Eyes.
    eye_size,
    eye_width,
    eye_height,
    eye_distance,
    eye_tilt,
    eye_position_y,
    inner_canthus,
    outer_canthus,
    lower_eyelid_down,
    pupil_scale,
    aegyo_sal,
    eye_brow_distance,
    eyelid_fold_width,
    // Eyebrows.
    eyebrow_height,
    eyebrow_distance,
    eyebrow_length,
    eyebrow_thickness,
    eyebrow_arch,
    eyebrow_tail,
    eyebrow_tilt,
    // Nose.
    nose_scale,
    nose_wing_width,
    nose_tip_size,
    nose_tip_angle,
    nose_length,
    bridge_width,
    bridge_curvature,
    nasal_hump_reduction,
    columella_height,
    radix_height,
    nostril_size,
    // Mouth.
    mouth_scale,
    mouth_width,
    mouth_position_y,
    philtrum_length,
    upper_lip_thickness,
    lower_lip_thickness,
    smile_corners,
    m_lip_shape,
    lip_plumpness,
    lip_protrusion,
    lower_lip_to_chin,
    teeth_align,
    // Proportions, symmetry, neck and depth.
    three_section_ratio,
    face_symmetry,
    eye_symmetry,
    eyebrow_symmetry,
    neck_slim,
    neck_length,
    e_line_adjustment,
    midface_projection,
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// Face Reshape panel state, persisted with the rest of [`crate::state::UiState`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FaceReshapeState {
    pub params: FaceReshapeParams,
    /// The live `Face Reshape Preview` layer, once the engine has made one.
    pub preview_layer: Option<u64>,
    /// The pixel layer the preview is built from.
    pub source_layer: Option<u64>,
    /// The open accordion, 0..5.
    pub active_group: usize,
    /// True while Compare is held (the engine hides the preview).
    pub compare: bool,
    pub status: String,
    pub status_error: bool,
    /// True while a command is in flight.
    #[serde(skip)]
    pub busy: bool,
    /// The sliders differ from what is applied.
    pub dirty: bool,
    /// The document these layer ids belong to; a switch clears the stale ids.
    pub document: Option<u64>,
    /// The drag generation used to build one coalesce key per slider drag.
    #[serde(skip)]
    pub gesture: u64,
}

impl FaceReshapeState {
    /// Clamp and round loaded values, and keep the accordion in range.
    pub fn ensure(&mut self) {
        for key in FaceReshapeParams::KEYS {
            if let Some(value) = self.params.get_mut(key) {
                *value = if value.is_finite() { value.clamp(-100.0, 100.0).round() } else { 0.0 };
            }
        }
        if self.active_group >= GROUPS.len() {
            self.active_group = 0;
        }
    }

    /// Zero every slider and forget the preview. Returns the preview id that was live.
    pub fn reset_all(&mut self) -> Option<u64> {
        let old = self.preview_layer.take();
        self.params = FaceReshapeParams::default();
        self.dirty = false;
        old
    }

    /// Zero one group and forget the preview. Returns the preview id that was live.
    pub fn reset_group(&mut self, group: usize) -> Option<u64> {
        let old = self.preview_layer.take();
        self.params.zero_group(group);
        self.dirty = !self.params.is_identity();
        old
    }
}

// ---------------------------------------------------------------------------------------------
// Command payloads
// ---------------------------------------------------------------------------------------------

fn command_payload(state: &FaceReshapeState) -> serde_json::Map<String, Value> {
    let mut payload = serde_json::Map::new();
    if let Some(source) = state.source_layer {
        payload.insert("layer".into(), json!(source));
    }
    if let Some(preview) = state.preview_layer {
        payload.insert("previewLayer".into(), json!(preview));
    }
    payload
}

fn preview_payload(state: &FaceReshapeState, coalesce: Option<&str>) -> Value {
    let mut payload = command_payload(state);
    payload.insert("params".into(), state.params.to_json());
    payload.insert("preview".into(), json!(true));
    if let Some(key) = coalesce {
        payload.insert("coalesce".into(), json!(key));
    }
    Value::Object(payload)
}

fn apply_payload(state: &FaceReshapeState) -> Value {
    let mut payload = command_payload(state);
    payload.insert("params".into(), state.params.to_json());
    payload.insert("preview".into(), json!(false));
    Value::Object(payload)
}

fn compare_payload(state: &FaceReshapeState, on: bool) -> Value {
    let mut payload = command_payload(state);
    payload.insert("compare".into(), json!(on));
    Value::Object(payload)
}

fn reset_payload(state: &FaceReshapeState, preview: Option<u64>) -> Value {
    let mut payload = serde_json::Map::new();
    if let Some(source) = state.source_layer {
        payload.insert("layer".into(), json!(source));
    }
    if let Some(preview) = preview {
        payload.insert("previewLayer".into(), json!(preview));
    }
    payload.insert("reset".into(), json!(true));
    Value::Object(payload)
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

enum Action {
    Preview { payload: Value },
    Apply { payload: Value },
    Compare { on: bool, payload: Value },
    Reset { payload: Value },
}

/// Draws Window › Face Reshape. Never needs a document: without one it shows the hint and closes.
pub fn window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.panels.face_reshape {
        return;
    }
    let has_document = app.session.active_index().is_some();
    let document = app.session.active().map(|st| st.doc.id.0);
    let state = &mut app.ui.face_reshape;
    state.ensure();
    if state.document != document {
        state.document = document;
        state.preview_layer = None;
        state.source_layer = None;
        state.compare = false;
        state.dirty = false;
        state.status.clear();
        state.status_error = false;
    }
    if has_document && state.preview_layer.is_none() {
        // Follow the layer the user selects until a preview exists; after that the preview's
        // source stays fixed so the next slider drag keeps warping the same pixels.
        state.source_layer = app.session.active().and_then(|st| st.active_layer).map(|id| id.0);
    }

    let mut open = true;
    let mut actions = Vec::new();
    let t = Tokens::get(ctx);
    let frame = card_frame(&t);
    let canvas = app.last_canvas_rect;
    let frame_margin = frame.total_margin().sum().y;
    let default_top = canvas.top() + 8.0;
    let panel_top = ctx.memory(|mem| mem.area_rect(panel_id())).map_or(default_top, |rect| rect.top());
    let room = (ctx.content_rect().bottom() - panel_top.min(default_top) - frame_margin - HEADER_HEIGHT).max(0.0);
    let body_height = BODY_HEIGHT.min(room);
    egui::Window::new(entry_label())
        .id(panel_id())
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(frame)
        .default_pos(egui::pos2((canvas.right() - WIDTH - 30.0).max(canvas.left() + 8.0), default_top))
        .min_height(HEADER_HEIGHT + body_height + frame_margin)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(entry_label()).font(theme::semibold(13.0)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 20.0, false, close_label()).clicked() {
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
                ui.painter().text(egui::pos2(rect.left() + 8.0, rect.center().y), Align2::LEFT_CENTER, open_hint(), theme::medium(11.5), t.warning);
                let hint = ui.interact(rect, ui.id().with("face-reshape-no-document"), Sense::hover());
                hint.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, open_hint()));
                return;
            }

            top_controls(ui, state, &mut actions);
            let body = egui::ScrollArea::vertical().id_salt("face-reshape-body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, keys) in GROUPS.iter().enumerate() {
                    let is_open = state.active_group == index;
                    if group_header(ui, index, is_open) {
                        state.active_group = index;
                    }
                    if is_open {
                        for key in *keys {
                            let Some(value) = state.params.get_mut(key) else { continue };
                            let outcome = slider_row(ui, key, value);
                            if outcome.drag_started {
                                state.gesture = state.gesture.wrapping_add(1);
                            }
                            if outcome.reset || outcome.changed {
                                state.dirty = true;
                                let coalesce = outcome.dragging.then(|| format!("face-reshape:{key}:{}", state.gesture));
                                actions.push(Action::Preview { payload: preview_payload(state, coalesce.as_deref()) });
                            }
                        }
                        ui.add_space(4.0);
                    }
                }
            });
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(panel_id().with("body"), (body.inner_rect, body.content_size)));
            #[cfg(not(test))]
            let _ = body;

            ui.add_space(4.0);
            widgets::hairline(ui);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, apply_label(), 120.0).on_hover_text(apply_hint()).clicked() {
                    actions.push(Action::Apply { payload: apply_payload(state) });
                }
            });
            if state.busy {
                ui.add(egui::Label::new(egui::RichText::new(busy_label()).size(11.0).color(t.text_dim)).wrap());
            } else if !state.status.is_empty() {
                let color = if state.status_error { t.danger } else { t.text_dim };
                ui.add(egui::Label::new(egui::RichText::new(&state.status).size(11.0).color(color)).wrap());
            }
        });

    if !open {
        if app.ui.face_reshape.compare {
            let payload = compare_payload(&app.ui.face_reshape, false);
            actions.push(Action::Compare { on: false, payload });
        }
        app.ui.panels.face_reshape = false;
    }
    for action in actions {
        apply(app, action);
    }
}

fn top_controls(ui: &mut egui::Ui, state: &mut FaceReshapeState, actions: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        if widgets::secondary_button(ui, reset_all_label(), 0.0).clicked() {
            let preview = state.preview_layer;
            let payload = reset_payload(state, preview);
            state.reset_all();
            actions.push(Action::Reset { payload });
        }
        if widgets::secondary_button(ui, reset_group_label(), 0.0).clicked() {
            let preview = state.preview_layer;
            let payload = reset_payload(state, preview);
            state.reset_group(state.active_group);
            actions.push(Action::Reset { payload });
        }
        let compare = ui.add(egui::Button::new(compare_label())).on_hover_text(compare_hint());
        let down = compare.is_pointer_button_down_on();
        if down && !state.compare {
            state.compare = true;
            actions.push(Action::Compare { on: true, payload: compare_payload(state, true) });
        } else if !down && state.compare {
            state.compare = false;
            actions.push(Action::Compare { on: false, payload: compare_payload(state, false) });
        }
    });
    ui.add_space(4.0);
}

fn group_header(ui: &mut egui::Ui, index: usize, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    widgets::hairline(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let chevron = egui::Rect::from_center_size(egui::pos2(rect.left() + 7.0, rect.center().y), vec2(12.0, 12.0));
    icons::paint(ui, chevron, if open { "chevron-down" } else { "chevron-right" }, 11.0, t.text_dim);
    let title = group_title(index);
    ui.painter().text(egui::pos2(rect.left() + 18.0, rect.center().y), Align2::LEFT_CENTER, title, theme::semibold(12.0), t.text);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    if response.clicked() {
        return true;
    }
    false
}

#[derive(Default)]
struct RowOutcome {
    changed: bool,
    reset: bool,
    drag_started: bool,
    dragging: bool,
}

fn slider_row(ui: &mut egui::Ui, key: &'static str, value: &mut f32) -> RowOutcome {
    let t = Tokens::get(ui.ctx());
    let mut out = RowOutcome::default();
    let text = label(key);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().interact_size.y = 20.0;
        let label_response =
            ui.add_sized([LABEL_WIDTH, 20.0], egui::Label::new(egui::RichText::new(text).size(11.0).color(t.text_dim)).truncate().sense(Sense::click()));
        if label_response.double_clicked() {
            *value = 0.0;
            out.reset = true;
        }
        let field = FIELD_WIDTH;
        let slider_width = (ui.available_width() - field - ui.spacing().item_spacing.x).max(60.0);
        ui.spacing_mut().slider_width = slider_width;
        let slider = ui.add(egui::Slider::new(value, -100.0..=100.0).step_by(1.0).show_value(false)).labelled_by(label_response.id);
        if slider.double_clicked() {
            *value = 0.0;
            out.reset = true;
        }
        out.drag_started |= slider.drag_started();
        out.dragging |= slider.dragged();
        out.changed |= slider.changed();
        let before = *value;
        let number = ui.add_sized([field, 20.0], egui::DragValue::new(value).range(-100.0..=100.0).speed(1.0).max_decimals(0)).labelled_by(label_response.id);
        out.drag_started |= number.drag_started();
        out.dragging |= number.dragged();
        out.changed |= number.changed();
        if number.double_clicked() {
            *value = 0.0;
            out.reset = true;
        }
        if *value != before {
            out.changed = true;
        }
    });
    *value = if value.is_finite() { value.clamp(-100.0, 100.0).round() } else { 0.0 };
    out
}

fn set_status(state: &mut FaceReshapeState, message: String, error: bool) {
    state.status = message;
    state.status_error = error;
}

fn apply(app: &mut PhotocraftApp, action: Action) {
    match action {
        Action::Preview { payload } => {
            app.ui.face_reshape.busy = true;
            let result = app.run(CMD, payload);
            app.ui.face_reshape.busy = false;
            let state = &mut app.ui.face_reshape;
            match result {
                Ok(value) => {
                    state.preview_layer = value.get("layer").and_then(Value::as_u64);
                    state.compare = value.get("compare").and_then(Value::as_bool).unwrap_or(false);
                    state.dirty = !state.params.is_identity();
                    if state.preview_layer.is_none() {
                        state.dirty = false;
                        set_status(state, preview_cleared().to_string(), false);
                    } else {
                        let points = value.get("points").and_then(Value::as_u64).unwrap_or(0);
                        let message = crate::i18n::fmt(preview_updated(), &[("points", &points.to_string())]);
                        set_status(state, message, false);
                    }
                }
                Err(error) => set_status(state, crate::i18n::fmt(preview_failed(), &[("error", &error)]), true),
            }
        }
        Action::Apply { payload } => {
            app.ui.face_reshape.busy = true;
            let result = app.run(CMD, payload);
            app.ui.face_reshape.busy = false;
            let state = &mut app.ui.face_reshape;
            match result {
                Ok(_) => {
                    state.preview_layer = None;
                    state.compare = false;
                    state.dirty = false;
                    set_status(state, applied().to_string(), false);
                }
                Err(error) => set_status(state, crate::i18n::fmt(apply_failed(), &[("error", &error)]), true),
            }
        }
        Action::Compare { on, payload } => {
            let result = app.run(CMD, payload);
            let state = &mut app.ui.face_reshape;
            match result {
                Ok(_) => state.compare = on,
                Err(error) => {
                    state.compare = false;
                    set_status(state, crate::i18n::fmt(compare_failed(), &[("error", &error)]), true);
                }
            }
        }
        Action::Reset { payload } => {
            let result = app.run(CMD, payload);
            let state = &mut app.ui.face_reshape;
            match result {
                Ok(_) => {
                    state.preview_layer = None;
                    state.compare = false;
                    state.dirty = !state.params.is_identity();
                    set_status(state, reset_done().to_string(), false);
                }
                Err(error) => set_status(state, crate::i18n::fmt(reset_failed(), &[("error", &error)]), true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::Pos2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
    }

    fn draw(app: PhotocraftApp, size: egui::Vec2, step_dt: f32) -> Harness<'static, PhotocraftApp> {
        let mut h = Harness::builder().with_size(size).with_step_dt(step_dt).build_ui_state(
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

    fn document_app() -> PhotocraftApp {
        let mut app = app();
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app.ui.panels.face_reshape = true;
        app
    }

    fn body_size(h: &Harness<'_, PhotocraftApp>) -> (egui::Rect, egui::Vec2) {
        h.ctx.data(|d| d.get_temp(panel_id().with("body"))).expect("the body stored its viewport and content size")
    }

    fn clicks_with(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, clicks: usize) {
        h.run_steps(40);
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..clicks {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
        }
        h.run_steps(3);
    }

    #[test]
    fn panel_draws_without_a_document_and_closes() {
        let mut app = app();
        app.ui.panels.face_reshape = true;
        let mut h = draw(app, vec2(900.0, 800.0), 0.25);
        assert!(h.query_all_by_label(open_hint()).next().is_some(), "the no-document hint is shown");
        assert!(h.state().ui.panels.face_reshape, "the panel stays open");
        assert_eq!(h.query_all_by_role(egui::accesskit::Role::Slider).count(), 0, "no sliders without a document");

        h.get_by_label(close_label()).click();
        h.run_steps(3);
        assert!(!h.state().ui.panels.face_reshape, "the close button closes the panel");
    }

    #[test]
    fn params_cover_the_engine_keys_in_order_and_round_trip() {
        assert_eq!(FaceReshapeParams::KEYS.len(), 68);
        assert_eq!(FaceReshapeParams::COUNT, 68);
        let mut flat = Vec::new();
        for keys in GROUPS {
            flat.extend_from_slice(keys);
        }
        assert_eq!(flat, FaceReshapeParams::KEYS, "groups follow the engine order");

        let mut params = FaceReshapeParams::default();
        assert!(params.is_identity(), "every slider starts at 0");
        for key in FaceReshapeParams::KEYS {
            assert_eq!(params.get(key), Some(0.0), "{key} defaults to 0");
        }
        params.set("nose_tip_angle", -73.0);
        params.set("jaw_width", 120.0);
        assert_eq!(params.get("jaw_width"), Some(100.0), "values clamp to 100");
        let back: FaceReshapeParams = serde_json::from_str(&serde_json::to_string(&params).expect("serialize")).expect("deserialize");
        assert_eq!(back, params, "params round-trip through serde");

        let mut state = FaceReshapeState { active_group: 4, compare: true, dirty: true, preview_layer: Some(7), source_layer: Some(3), ..Default::default() };
        state.params.set("lip_plumpness", 21.0);
        state.status = "Preview updated".into();
        let back: FaceReshapeState = serde_json::from_str(&serde_json::to_string(&state).expect("serialize")).expect("deserialize");
        assert_eq!(back, state, "panel state round-trips");
    }

    #[test]
    fn reset_group_and_all_zero_fields_and_clear_the_preview() {
        let mut state = FaceReshapeState::default();
        for (index, key) in FaceReshapeParams::KEYS.iter().enumerate() {
            state.params.set(key, (index % 31) as f32 - 15.0);
        }
        state.preview_layer = Some(9);
        state.dirty = true;

        let first_eye = state.params.get("eye_size");
        let old = state.reset_group(0);
        assert_eq!(old, Some(9), "the preview id is returned");
        assert_eq!(state.preview_layer, None, "the preview is forgotten");
        for key in GROUPS[0] {
            assert_eq!(state.params.get(key), Some(0.0), "{key} is zeroed");
        }
        assert_eq!(state.params.get("eye_size"), first_eye, "another group is untouched");
        assert!(state.dirty, "remaining non-zero values are still a pending edit");

        state.preview_layer = Some(10);
        let old = state.reset_all();
        assert_eq!(old, Some(10));
        assert!(state.params.is_identity(), "Reset All zeroes every slider");
        assert_eq!(state.preview_layer, None);
        assert!(!state.dirty);
    }

    #[test]
    fn preview_apply_compare_and_reset_payloads_use_the_engine_contract() {
        let mut state = FaceReshapeState { source_layer: Some(3), preview_layer: Some(4), ..Default::default() };
        state.params.set("jaw_width", 25.0);
        state.params.set("lip_plumpness", -12.0);

        let preview = preview_payload(&state, Some("face-reshape:jaw_width:7"));
        assert_eq!(preview["layer"], 3);
        assert_eq!(preview["previewLayer"], 4);
        assert_eq!(preview["preview"], true);
        assert_eq!(preview["coalesce"], "face-reshape:jaw_width:7");
        assert_eq!(preview["params"].as_object().map(serde_json::Map::len), Some(68), "all 68 keys go to the engine");
        assert_eq!(preview["params"]["jaw_width"], 25.0);
        assert_eq!(preview["params"]["lip_plumpness"], -12.0);

        let apply = apply_payload(&state);
        assert_eq!(apply["preview"], false, "Apply asks for the final layer");
        assert_eq!(apply["layer"], 3);
        assert_eq!(apply["previewLayer"], 4);
        assert_eq!(apply["params"]["jaw_width"], 25.0);

        let compare = compare_payload(&state, true);
        assert_eq!(compare["compare"], true);
        assert_eq!(compare["previewLayer"], 4);
        assert!(compare.get("params").is_none(), "Compare is a preview-only call");
        let release = compare_payload(&state, false);
        assert_eq!(release["compare"], false);

        let reset = reset_payload(&state, Some(4));
        assert_eq!(reset["reset"], true);
        assert_eq!(reset["previewLayer"], 4);
        assert!(reset.get("params").is_none(), "Reset needs no sliders");
    }

    #[test]
    fn vietnamese_and_english_labels_come_from_the_local_table() {
        assert_eq!(entry_label(), "Face Reshape");
        assert_eq!(group_title(0), "Face & Head Shape");
        assert_eq!(group_title(5), "Proportions & Neck");
        assert_eq!(label("head_scale"), "Head scale");
        assert_eq!(label("double_chin_warp"), "Double chin");

        let vi = crate::i18n::Lang::from_code("vi").expect("vi is registered");
        crate::i18n::with_language(vi, || {
            assert_eq!(entry_label(), "Định hình khuôn mặt");
            assert_eq!(close_label(), "Đóng");
            assert_eq!(group_title(0), "Khung mặt & Đầu");
            assert_eq!(group_title(1), "Mắt & Tròng mắt");
            assert_eq!(group_title(2), "Lông mày");
            assert_eq!(group_title(3), "Mũi");
            assert_eq!(group_title(4), "Miệng & Môi");
            assert_eq!(group_title(5), "Tỷ lệ & Cổ");
            assert_eq!(label("head_scale"), "Tỷ lệ đầu");
            assert_eq!(label("eye_size"), "Kích thước mắt");
            assert_eq!(label("eyebrow_arch"), "Độ cong lông mày");
            assert_eq!(label("nose_tip_angle"), "Góc đầu mũi");
            assert_eq!(label("upper_lip_thickness"), "Độ dày môi trên");
            assert_eq!(label("neck_slim"), "Thu gọn cổ");
        });
        for key in FaceReshapeParams::KEYS {
            assert!(!label(key).is_empty(), "no label for {key}");
            assert!(LABELS.iter().any(|(k, _, _)| k == key), "no table row for {key}");
        }
    }

    #[test]
    fn every_group_shows_only_its_own_sliders_and_the_body_fits_or_scrolls() {
        let mut h = draw(document_app(), vec2(1000.0, 900.0), 0.25);
        let mut total = 0;
        for (index, keys) in GROUPS.iter().enumerate() {
            h.state_mut().ui.face_reshape.active_group = index;
            h.run_steps(3);
            let shown = h.query_all_by_role(egui::accesskit::Role::Slider).count();
            assert_eq!(shown, keys.len(), "group {index} shows exactly its sliders");
            total += shown;
        }
        assert_eq!(total, 68, "all 68 sliders exist across the six groups");

        h.state_mut().ui.face_reshape.active_group = 0;
        h.run_steps(3);
        let (viewport, content) = body_size(&h);
        assert!(content.y <= viewport.height() + 1.0, "the tallest group fits at 900 points: {content:?} in {viewport:?}");

        let mut h = draw(document_app(), vec2(1000.0, 360.0), 0.25);
        h.state_mut().ui.face_reshape.active_group = 0;
        h.run_steps(3);
        let (viewport, content) = body_size(&h);
        assert!(content.y > viewport.height(), "a short window scrolls instead of clipping: {content:?} in {viewport:?}");
    }

    #[test]
    fn double_click_on_the_label_or_slider_resets_the_field() {
        let mut app = document_app();
        app.ui.face_reshape.params.head_scale = 37.0;
        let mut h = draw(app, vec2(1000.0, 900.0), 1.0 / 60.0);

        let slider_rect = h.query_all_by_role(egui::accesskit::Role::Slider).next().expect("the first slider").rect();
        // The label column ends just left of the slider.
        let label_center = Pos2::new(slider_rect.left() - 20.0, slider_rect.center().y);
        clicks_with(&mut h, label_center, 2);
        assert_eq!(h.state().ui.face_reshape.params.head_scale, 0.0, "double-click on the label resets");

        h.state_mut().ui.face_reshape.params.head_scale = 37.0;
        h.run_steps(2);
        let slider_rect = h.query_all_by_role(egui::accesskit::Role::Slider).next().expect("the first slider").rect();
        clicks_with(&mut h, slider_rect.center(), 2);
        assert_eq!(h.state().ui.face_reshape.params.head_scale, 0.0, "double-click on the slider resets");
    }
}

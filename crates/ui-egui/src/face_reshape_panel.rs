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

use egui::{Align2, CornerRadius, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

/// The engine command this panel drives.
pub const CMD: &str = "retouchAI.faceReshape";
/// Panel width in points.
pub const WIDTH: f32 = 430.0;
/// The chrome above the scrolling body (the title/subtitle row, the hairline and their spacing).
const HEADER_HEIGHT: f32 = 57.0;
/// The preset and face cards above the body, with the space below them. The extra row is the
/// numbered-face checkboxes, kept in the budget so switching to Selected never clips the actions.
const TOP_CHROME: f32 = 256.0;
/// The sticky action card below the body, with the space above it and room for the status line.
const BOTTOM_CHROME: f32 = 125.0;
/// The body height when there is room for all 17 sliders of the widest group.
pub const BODY_HEIGHT: f32 = 760.0;
/// The label column of a slider row.
const LABEL_WIDTH: f32 = 150.0;
/// The numeric field of a slider row.
const FIELD_WIDTH: f32 = 56.0;
/// The height of one slider row.
const ROW_HEIGHT: f32 = 22.0;

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

fn face_mode_label() -> &'static str {
    pick("Faces", "Khuôn mặt")
}

fn face_main_label() -> &'static str {
    pick("Main", "Chính")
}

fn face_all_label() -> &'static str {
    pick("All", "Tất cả")
}

fn face_selected_label() -> &'static str {
    pick("Selected", "Đã chọn")
}

fn face_number_label() -> &'static str {
    pick("Face {n}", "Mặt {n}")
}

fn preset_label() -> &'static str {
    pick("Preset", "Preset")
}

fn subtitle() -> &'static str {
    pick("Retouch AI · Face sculpt", "Retouch AI · Định hình khuôn mặt")
}

fn save_preset_label() -> &'static str {
    pick("Save Current", "Lưu hiện tại")
}

fn preset_name_hint() -> &'static str {
    pick("Preset name", "Tên preset")
}

fn delete_preset_label() -> &'static str {
    pick("Delete", "Xóa")
}

fn preset_saved() -> &'static str {
    pick("Preset \"{name}\" saved.", "Đã lưu preset \"{name}\".")
}

fn preset_deleted() -> &'static str {
    pick("Preset \"{name}\" deleted.", "Đã xóa preset \"{name}\".")
}

fn preset_empty() -> &'static str {
    pick("Name the preset before saving.", "Hãy đặt tên preset trước khi lưu.")
}

fn custom_preset_label() -> &'static str {
    pick("Custom", "Tùy chỉnh")
}

fn preset_missing() -> &'static str {
    pick("That preset no longer exists.", "Preset đó không còn tồn tại.")
}

const PRESET_NAMES: [(&str, &str, &str); 6] = [
    ("natural", "Natural", "Tự nhiên"),
    ("vLine", "V-Line", "V-Line"),
    ("bigEyes", "Big Eyes", "Mắt to"),
    ("slimNose", "Slim Nose", "Mũi thon"),
    ("fullLips", "Full Lips", "Môi đầy"),
    ("profileBalance", "Profile Balance", "Cân bằng nghiêng"),
];

fn preset_name(key: &str) -> &'static str {
    PRESET_NAMES.iter().find(|(k, _, _)| *k == key).map_or(custom_preset_label(), |(_, en, vi)| pick(en, vi))
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
// Presets
// ---------------------------------------------------------------------------------------------

/// Built-in looks: a key, its English/Vietnamese name in [`PRESET_NAMES`], and the non-zero
/// sliders. Every other slider stays 0. Keys are internal; the panel translates the labels.
pub const BUILTIN_PRESETS: [(&str, &[(&str, f32)]); 6] = [
    (
        "natural",
        &[
            ("face_slim", 8.0),
            ("face_width", 4.0),
            ("jaw_width", -5.0),
            ("cheek_slim", 6.0),
            ("eye_size", 6.0),
            ("eye_brow_distance", 3.0),
            ("nose_tip_size", -4.0),
            ("lip_plumpness", 6.0),
            ("face_symmetry", 8.0),
        ],
    ),
    (
        "vLine",
        &[
            ("face_slim", 25.0),
            ("face_length", 6.0),
            ("jaw_width", -22.0),
            ("jaw_angle", 18.0),
            ("cheek_slim", 15.0),
            ("chin_width", -12.0),
            ("chin_length", 8.0),
            ("face_symmetry", 10.0),
        ],
    ),
    ("bigEyes", &[("eye_size", 32.0), ("eye_width", 14.0), ("eye_height", 10.0), ("lower_eyelid_down", 8.0), ("aegyo_sal", 10.0), ("pupil_scale", 6.0)]),
    (
        "slimNose",
        &[
            ("nose_scale", -18.0),
            ("nose_wing_width", -20.0),
            ("nose_tip_size", -12.0),
            ("bridge_width", -14.0),
            ("nasal_hump_reduction", 10.0),
            ("nostril_size", -10.0),
        ],
    ),
    (
        "fullLips",
        &[
            ("mouth_scale", 8.0),
            ("mouth_width", 4.0),
            ("upper_lip_thickness", 20.0),
            ("lower_lip_thickness", 18.0),
            ("lip_plumpness", 30.0),
            ("m_lip_shape", 15.0),
            ("smile_corners", 8.0),
        ],
    ),
    (
        "profileBalance",
        &[
            ("nose_length", 8.0),
            ("bridge_curvature", -8.0),
            ("nasal_hump_reduction", 15.0),
            ("chin_protrusion", 12.0),
            ("chin_length", 6.0),
            ("e_line_adjustment", 10.0),
            ("midface_projection", 8.0),
            ("three_section_ratio", 5.0),
        ],
    ),
];

/// A built-in preset's full 68-slider set.
pub fn builtin_params(key: &str) -> Option<FaceReshapeParams> {
    let (_, values) = BUILTIN_PRESETS.iter().find(|(k, _)| *k == key)?;
    let mut params = FaceReshapeParams::default();
    for (key, value) in *values {
        params.set(key, *value);
    }
    Some(params)
}

/// A user preset saved from the panel: a name and the full 68-slider set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceReshapePreset {
    pub name: String,
    pub params: FaceReshapeParams,
}

// ---------------------------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------------------------

/// Which detected faces the preview/apply calls ask for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FaceMode {
    /// The best face (the engine's default).
    #[default]
    Main,
    /// Every detected face.
    All,
    /// The numbered faces in [`FaceReshapeState::selected_faces`].
    Selected,
}

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
    /// Main, every face, or the numbered selection below.
    pub face_mode: FaceMode,
    /// 1-based faces selected in [`FaceMode::Selected`], in the detector's order.
    pub selected_faces: Vec<usize>,
    /// How many faces the last preview/apply detected; checkboxes appear above one.
    pub detected_faces: usize,
    /// User presets saved from the panel (built-ins live in [`BUILTIN_PRESETS`]).
    pub presets: Vec<FaceReshapePreset>,
    /// The last preset picked, `builtin:<key>` or `user:<name>`, for the dropdown's check mark.
    pub last_preset: String,
    /// The name typed for Save Current.
    pub preset_name: String,
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
        self.selected_faces.retain(|n| *n >= 1);
        self.selected_faces.sort_unstable();
        self.selected_faces.dedup();
        if self.detected_faces > 0 {
            if self.face_mode == FaceMode::Selected && self.detected_faces < 2 {
                self.face_mode = FaceMode::Main;
            }
            self.selected_faces.retain(|n| *n <= self.detected_faces);
        }
    }

    /// The engine's `faces` value for preview/apply.
    pub fn faces_value(&self) -> Value {
        match self.face_mode {
            FaceMode::Main => json!("main"),
            FaceMode::All => json!("all"),
            FaceMode::Selected if self.selected_faces.is_empty() => json!("main"),
            FaceMode::Selected => json!(self.selected_faces),
        }
    }

    /// Take the detection count from an engine result and keep the selection valid.
    pub fn set_detected_faces(&mut self, count: usize) {
        self.detected_faces = count;
        if count < 2 {
            self.face_mode = FaceMode::Main;
            self.selected_faces.clear();
        } else {
            self.selected_faces.retain(|n| *n <= count);
            if self.face_mode == FaceMode::Selected && self.selected_faces.is_empty() {
                self.selected_faces = (1..=count).collect();
            }
        }
    }

    /// Whether face `n` is checked. A Selected mode with no stored numbers means every face.
    pub fn face_checked(&self, n: usize) -> bool {
        self.selected_faces.is_empty() || self.selected_faces.contains(&n)
    }

    /// Toggle one face in Selected mode; at least one face always stays checked.
    pub fn toggle_face(&mut self, n: usize, count: usize) {
        if count == 0 || n == 0 || n > count {
            return;
        }
        let mut selected = if self.selected_faces.is_empty() { (1..=count).collect() } else { self.selected_faces.clone() };
        if let Some(pos) = selected.iter().position(|x| *x == n) {
            if selected.len() == 1 {
                return;
            }
            selected.remove(pos);
        } else {
            selected.push(n);
        }
        selected.sort_unstable();
        selected.dedup();
        self.selected_faces = selected;
    }

    /// Apply a built-in preset. Returns true when `key` names one.
    pub fn apply_builtin(&mut self, key: &str) -> bool {
        let Some(params) = builtin_params(key) else { return false };
        self.params = params;
        self.last_preset = format!("builtin:{key}");
        self.dirty = true;
        true
    }

    /// Apply a saved user preset by name.
    pub fn apply_user_preset(&mut self, name: &str) -> bool {
        let Some(preset) = self.presets.iter().find(|preset| preset.name == name) else { return false };
        self.params = preset.params;
        self.last_preset = format!("user:{name}");
        self.dirty = true;
        true
    }

    /// Save the current 68 sliders as a named user preset, replacing a preset with the same name.
    pub fn save_preset(&mut self, name: &str) -> std::result::Result<(), &'static str> {
        let name = name.trim();
        if name.is_empty() {
            return Err("empty");
        }
        let preset = FaceReshapePreset { name: name.to_string(), params: self.params };
        match self.presets.iter_mut().find(|existing| existing.name == name) {
            Some(existing) => *existing = preset,
            None => self.presets.push(preset),
        }
        self.last_preset = format!("user:{name}");
        self.preset_name.clear();
        Ok(())
    }

    /// Delete the selected user preset. Returns its name when one was removed.
    pub fn delete_selected_preset(&mut self) -> Option<String> {
        let name = self.last_preset.strip_prefix("user:")?.to_string();
        let at = self.presets.iter().position(|preset| preset.name == name)?;
        self.presets.remove(at);
        self.last_preset.clear();
        Some(name)
    }

    /// The preset dropdown's text for the current selection.
    pub fn preset_label(&self) -> String {
        if let Some(key) = self.last_preset.strip_prefix("builtin:") {
            return preset_name(key).to_string();
        }
        if let Some(name) = self.last_preset.strip_prefix("user:") {
            return name.to_string();
        }
        custom_preset_label().to_string()
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
    payload.insert("faces".into(), state.faces_value());
    payload.insert("preview".into(), json!(true));
    if let Some(key) = coalesce {
        payload.insert("coalesce".into(), json!(key));
    }
    Value::Object(payload)
}

fn apply_payload(state: &FaceReshapeState) -> Value {
    let mut payload = command_payload(state);
    payload.insert("params".into(), state.params.to_json());
    payload.insert("faces".into(), state.faces_value());
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
        state.detected_faces = 0;
        state.selected_faces.clear();
        state.face_mode = FaceMode::Main;
    }
    if has_document && state.preview_layer.is_none() {
        // Follow the layer the user selects until a preview exists; after that the preview's
        // source stays fixed so the next slider drag keeps warping the same pixels.
        state.source_layer = app.session.active().and_then(|st| st.active_layer).map(|id| id.0);
    }

    let mut open = true;
    let mut shortcuts = false;
    let mut actions = Vec::new();
    let t = Tokens::get(ctx);
    let frame = card_frame(&t);
    let canvas = app.last_canvas_rect;
    let frame_margin = frame.total_margin().sum().y;
    let default_top = canvas.top() + 8.0;
    let panel_top = ctx.memory(|mem| mem.area_rect(panel_id())).map_or(default_top, |rect| rect.top());
    let room = (ctx.content_rect().bottom() - panel_top.min(default_top) - frame_margin - HEADER_HEIGHT - TOP_CHROME - BOTTOM_CHROME).max(0.0);
    let body_height = BODY_HEIGHT.min(room);
    let min_height = if has_document { HEADER_HEIGHT + TOP_CHROME + body_height + BOTTOM_CHROME + frame_margin } else { HEADER_HEIGHT + 62.0 + frame_margin };
    egui::Window::new(entry_label())
        .id(panel_id())
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .frame(frame)
        .default_pos(egui::pos2((canvas.right() - WIDTH - 30.0).max(canvas.left() + 8.0), default_top))
        .min_height(min_height)
        .show(ctx, |ui| {
            ui.set_width(WIDTH);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(egui::RichText::new(entry_label()).font(theme::semibold(14.0)).color(t.text));
                    ui.label(egui::RichText::new(subtitle()).size(11.5).color(t.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "x", 20.0, false, close_label()).clicked() {
                        open = false;
                    }
                    if widgets::secondary_button(ui, tl!("Keyboard Shortcuts…"), 0.0).clicked() {
                        shortcuts = true;
                    }
                });
            });
            ui.add_space(6.0);
            widgets::hairline(ui);
            ui.add_space(6.0);
            if !has_document {
                widgets::soft_card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(egui::RichText::new(open_hint()).size(12.0).color(t.warning)).wrap());
                });
                return;
            }

            preset_card(ui, state, &mut actions);
            face_choice_card(ui, state, &mut actions);
            ui.add_space(6.0);
            let body = egui::ScrollArea::vertical().id_salt("face-reshape-body").max_height(body_height).auto_shrink([false, true]).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 1.0;
                for (index, keys) in GROUPS.iter().enumerate() {
                    let is_open = state.active_group == index;
                    let mut open_group = false;
                    widgets::soft_card(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if group_header(ui, index, keys.len(), is_open) {
                            open_group = true;
                        }
                        if is_open {
                            ui.add_space(3.0);
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
                            ui.add_space(2.0);
                        }
                    });
                    if open_group {
                        state.active_group = index;
                    }
                    ui.add_space(6.0);
                }
            });
            #[cfg(test)]
            ui.ctx().data_mut(|d| d.insert_temp(panel_id().with("body"), (body.inner_rect, body.content_size)));
            #[cfg(not(test))]
            let _ = body;

            ui.add_space(4.0);
            widgets::hairline(ui);
            ui.add_space(4.0);
            action_card(ui, state, &mut actions);
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
    if shortcuts {
        crate::prefs_ui::open_shortcuts_filtered(app, ENTRY_EN);
    }
}

fn preset_card(ui: &mut egui::Ui, state: &mut FaceReshapeState, actions: &mut Vec<Action>) {
    let mut chosen: Option<String> = None;
    widgets::soft_card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_header(ui, preset_label(), |ui| {
            let can_delete = state.last_preset.starts_with("user:");
            let delete = ui.add_enabled_ui(can_delete, |ui| widgets::secondary_button(ui, delete_preset_label(), 0.0)).inner;
            if delete.clicked()
                && let Some(name) = state.delete_selected_preset()
            {
                let message = crate::i18n::fmt(preset_deleted(), &[("name", &name)]);
                set_status(state, message, false);
            }
        });
        let mut label = state.preset_label();
        if label.is_empty() {
            label = custom_preset_label().to_string();
        }
        egui::ComboBox::from_id_salt("face-reshape-preset").selected_text(label).width(ui.available_width()).icon(widgets::chevron_icon).show_ui(ui, |ui| {
            if ui.selectable_label(state.last_preset.is_empty(), custom_preset_label()).clicked() {
                chosen = Some(String::new());
            }
            for (key, _) in BUILTIN_PRESETS {
                if ui.selectable_label(state.last_preset == format!("builtin:{key}"), preset_name(key)).clicked() {
                    chosen = Some(format!("builtin:{key}"));
                }
            }
            for preset in &state.presets {
                if ui.selectable_label(state.last_preset == format!("user:{}", preset.name), &preset.name).clicked() {
                    chosen = Some(format!("user:{}", preset.name));
                }
            }
        });
        if let Some(key) = chosen.take() {
            if key.is_empty() {
                state.last_preset.clear();
            } else if let Some(builtin) = key.strip_prefix("builtin:") {
                if state.apply_builtin(builtin) {
                    actions.push(Action::Preview { payload: preview_payload(state, None) });
                }
            } else if let Some(name) = key.strip_prefix("user:") {
                if state.apply_user_preset(name) {
                    actions.push(Action::Preview { payload: preview_payload(state, None) });
                } else {
                    set_status(state, preset_missing().to_string(), true);
                }
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let gap = ui.spacing().item_spacing.x;
            let save_w = widgets::button_width(ui, &[save_preset_label()]);
            let width = (ui.available_width() - save_w - gap).max(80.0);
            ui.add_sized([width, 26.0], egui::TextEdit::singleline(&mut state.preset_name).hint_text(preset_name_hint()));
            if widgets::secondary_button(ui, save_preset_label(), save_w).clicked() {
                match state.save_preset(&state.preset_name.clone()) {
                    Ok(()) => {
                        let name = state.last_preset.strip_prefix("user:").unwrap_or_default().to_string();
                        let message = crate::i18n::fmt(preset_saved(), &[("name", &name)]);
                        set_status(state, message, false);
                    }
                    Err(_) => set_status(state, preset_empty().to_string(), true),
                }
            }
        });
    });
    ui.add_space(6.0);
}

fn face_choice_card(ui: &mut egui::Ui, state: &mut FaceReshapeState, actions: &mut Vec<Action>) {
    let before = (state.face_mode, state.selected_faces.clone());
    widgets::soft_card(ui, |ui| {
        ui.set_width(ui.available_width());
        widgets::section_header(ui, face_mode_label(), |_| {});
        let current = match state.face_mode {
            FaceMode::Main => 0,
            FaceMode::All => 1,
            FaceMode::Selected => 2,
        };
        let labels = [face_main_label(), face_all_label(), face_selected_label()];
        let chosen = widgets::segmented_tabs(ui, ui.id().with("face-reshape-face-mode"), &labels, current);
        // Selected needs at least two detected faces, as before; an unavailable click is ignored.
        if chosen != current && !(chosen == 2 && state.detected_faces <= 1) {
            state.face_mode = match chosen {
                0 => FaceMode::Main,
                1 => FaceMode::All,
                _ => FaceMode::Selected,
            };
            if state.face_mode == FaceMode::Selected && state.selected_faces.is_empty() && state.detected_faces > 0 {
                state.selected_faces = (1..=state.detected_faces).collect();
            }
        }
        if state.face_mode == FaceMode::Selected && state.detected_faces > 1 {
            ui.add_space(3.0);
            ui.horizontal_wrapped(|ui| {
                for n in 1..=state.detected_faces {
                    let mut on = state.face_checked(n);
                    let label = crate::i18n::fmt(face_number_label(), &[("n", &n.to_string())]);
                    if widgets::checkbox(ui, &mut on, &label).changed() {
                        state.toggle_face(n, state.detected_faces);
                    }
                }
            });
        }
    });
    let changed = before.0 != state.face_mode || before.1 != state.selected_faces;
    // Re-preview a live deformation so the chosen faces show at once; an all-zero set has
    // nothing to warp and only records the choice.
    if changed && !state.params.is_identity() {
        actions.push(Action::Preview { payload: preview_payload(state, None) });
    }
    ui.add_space(6.0);
}

/// Compare, Reset All, Reset Group and Apply stay visible below the scrolling sliders, so a long
/// group never hides the final action.
fn action_card(ui: &mut egui::Ui, state: &mut FaceReshapeState, actions: &mut Vec<Action>) {
    let t = Tokens::get(ui.ctx());
    widgets::soft_card(ui, |ui| {
        ui.set_width(ui.available_width());
        let gap = ui.spacing().item_spacing.x;
        let compare_w = widgets::button_width(ui, &[compare_label()]);
        let reset_all_w = widgets::button_width(ui, &[reset_all_label()]);
        let reset_group_w = widgets::button_width(ui, &[reset_group_label()]);
        let widths = [compare_w, reset_all_w, reset_group_w];
        let total: f32 = widths.iter().sum::<f32>() + 2.0 * gap;
        let scale = if total > ui.available_width() && total > 0.0 { ui.available_width() / total } else { 1.0 };
        ui.horizontal(|ui| {
            let compare = widgets::secondary_button(ui, compare_label(), compare_w * scale).on_hover_text(compare_hint());
            let down = compare.is_pointer_button_down_on();
            if down && !state.compare {
                state.compare = true;
                actions.push(Action::Compare { on: true, payload: compare_payload(state, true) });
            } else if !down && state.compare {
                state.compare = false;
                actions.push(Action::Compare { on: false, payload: compare_payload(state, false) });
            }
            if widgets::secondary_button(ui, reset_all_label(), reset_all_w * scale).clicked() {
                let preview = state.preview_layer;
                let payload = reset_payload(state, preview);
                state.reset_all();
                actions.push(Action::Reset { payload });
            }
            if widgets::secondary_button(ui, reset_group_label(), reset_group_w * scale).clicked() {
                let preview = state.preview_layer;
                let payload = reset_payload(state, preview);
                state.reset_group(state.active_group);
                actions.push(Action::Reset { payload });
            }
        });
        ui.add_space(4.0);
        if widgets::primary_button(ui, apply_label(), ui.available_width()).on_hover_text(apply_hint()).clicked() {
            actions.push(Action::Apply { payload: apply_payload(state) });
        }
        if state.busy {
            ui.add_space(3.0);
            ui.add(egui::Label::new(egui::RichText::new(busy_label()).size(11.5).color(t.text_dim)).wrap());
        } else if !state.status.is_empty() {
            ui.add_space(3.0);
            widgets::status_note(ui, &state.status, state.status_error);
        }
    });
}

fn group_header(ui: &mut egui::Ui, index: usize, count: usize, open: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(rect, t.radius_sm, t.hover.gamma_multiply(0.5));
    }
    let chevron = egui::Rect::from_center_size(egui::pos2(rect.left() + 8.0, rect.center().y), vec2(14.0, 14.0));
    icons::paint(ui, chevron, if open { "chevron-down" } else { "chevron-right" }, 12.0, t.text_dim);
    let title = group_title(index);
    ui.painter().text(egui::pos2(rect.left() + 24.0, rect.center().y), Align2::LEFT_CENTER, title, theme::semibold(12.5), t.text);
    ui.painter().text(egui::pos2(rect.right() - 4.0, rect.center().y), Align2::RIGHT_CENTER, count.to_string(), theme::mono(10.5), t.text_faint);
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::CollapsingHeader, true, open, title));
    response.clicked()
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
        ui.spacing_mut().interact_size.y = ROW_HEIGHT;
        let (label_rect, label_response) = ui.allocate_exact_size(vec2(LABEL_WIDTH, ROW_HEIGHT), Sense::click());
        let galley = crate::tab_strip::elided(ui, text, theme::medium(12.0), t.text_dim, LABEL_WIDTH - 4.0);
        let size = galley.size();
        ui.painter().galley(pos2(label_rect.right() - 2.0 - size.x, label_rect.center().y - size.y / 2.0), galley, t.text_dim);
        label_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
        let label_response = if label_response.hovered() { label_response.on_hover_text(text) } else { label_response };
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
        let number = ui
            .scope(|ui| {
                ui.style_mut().override_font_id = Some(theme::mono(12.0));
                ui.visuals_mut().override_text_color = Some(t.text);
                ui.visuals_mut().extreme_bg_color = t.field;
                ui.visuals_mut().text_edit_bg_color = Some(t.field);
                ui.visuals_mut().widgets.inactive.bg_fill = t.field;
                ui.visuals_mut().widgets.inactive.weak_bg_fill = t.field;
                ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, t.field_border);
                ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(1.0, t.text);
                ui.visuals_mut().widgets.hovered.bg_fill = t.hover;
                ui.visuals_mut().widgets.hovered.weak_bg_fill = t.hover;
                ui.visuals_mut().widgets.hovered.bg_stroke = Stroke::new(1.0, t.field_border);
                ui.visuals_mut().widgets.hovered.fg_stroke = Stroke::new(1.0, t.text);
                ui.visuals_mut().widgets.active.bg_fill = t.pressed;
                ui.visuals_mut().widgets.active.fg_stroke = Stroke::new(1.0, t.text);
                ui.add_sized([field, ROW_HEIGHT], egui::DragValue::new(value).range(-100.0..=100.0).speed(1.0).max_decimals(0).fixed_decimals(0))
            })
            .inner
            .labelled_by(label_response.id);
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
                    if let Some(detected) = value.get("detected").and_then(Value::as_u64) {
                        state.set_detected_faces(detected as usize);
                    }
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
                Ok(value) => {
                    if let Some(detected) = value.get("detected").and_then(Value::as_u64) {
                        state.set_detected_faces(detected as usize);
                    }
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

/// The panel's commands: Window › Face Reshape and `faceReshape.apply` (Edit › Keyboard
/// Shortcuts). Toggle flips the window; Apply runs the panel's own Apply payload, opening the
/// window first so its status line shows the result whichever way it was called.
pub(crate) fn menu(app: &mut PhotocraftApp, id: &str) -> Option<Result<Value, String>> {
    match id {
        "window.toggle.faceReshape" => {
            app.ui.panels.face_reshape = !app.ui.panels.face_reshape;
            Some(Ok(json!({"open": app.ui.panels.face_reshape})))
        }
        "faceReshape.apply" => {
            app.ui.panels.face_reshape = true;
            let document = app.session.active().map(|st| st.doc.id.0);
            let active = app.session.active().and_then(|st| st.active_layer).map(|id| id.0);
            {
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
                    state.detected_faces = 0;
                    state.selected_faces.clear();
                    state.face_mode = FaceMode::Main;
                }
                if state.source_layer.is_none() {
                    state.source_layer = active;
                }
            }
            if app.session.active_index().is_none() {
                set_status(&mut app.ui.face_reshape, open_hint().to_string(), true);
                return Some(Ok(json!({"action": "apply"})));
            }
            let payload = apply_payload(&app.ui.face_reshape);
            apply(app, Action::Apply { payload });
            Some(Ok(json!({"action": "apply"})))
        }
        _ => None,
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

    /// The header's Shortcuts… button opens Edit › Keyboard Shortcuts with "Face Reshape" in the
    /// search field.
    #[test]
    fn header_shortcuts_button_opens_keyboard_shortcuts_filtered_to_face_reshape() {
        let mut h = draw(document_app(), vec2(900.0, 800.0), 0.25);
        h.get_by_label("Keyboard Shortcuts…").click();
        h.run_steps(2);
        let dialog = h
            .state()
            .ui
            .dialogs
            .iter()
            .find(|dialog| dialog.fields.get("__prefsui").and_then(Value::as_str) == Some("shortcuts"))
            .expect("the Keyboard Shortcuts dialog opened");
        assert_eq!(dialog.fields.get("filter").and_then(Value::as_str), Some(ENTRY_EN));
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
        assert_eq!(preview["faces"], "main", "Main is the engine's default face choice");

        let apply = apply_payload(&state);
        assert_eq!(apply["preview"], false, "Apply asks for the final layer");
        assert_eq!(apply["layer"], 3);
        assert_eq!(apply["previewLayer"], 4);
        assert_eq!(apply["params"]["jaw_width"], 25.0);
        assert_eq!(apply["faces"], "main");

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
        assert_eq!(face_mode_label(), "Faces");
        assert_eq!(face_main_label(), "Main");
        assert_eq!(face_all_label(), "All");
        assert_eq!(face_selected_label(), "Selected");
        assert_eq!(preset_label(), "Preset");
        assert_eq!(save_preset_label(), "Save Current");
        assert_eq!(preset_name("natural"), "Natural");
        assert_eq!(preset_name("profileBalance"), "Profile Balance");
        assert_eq!(crate::i18n::tr(crate::i18n::Lang::EN, "Face Reshape"), "Face Reshape");

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
            assert_eq!(face_mode_label(), "Khuôn mặt");
            assert_eq!(face_main_label(), "Chính");
            assert_eq!(face_all_label(), "Tất cả");
            assert_eq!(face_selected_label(), "Đã chọn");
            assert_eq!(face_number_label(), "Mặt {n}");
            assert_eq!(preset_label(), "Preset");
            assert_eq!(save_preset_label(), "Lưu hiện tại");
            assert_eq!(preset_name("natural"), "Tự nhiên");
            assert_eq!(preset_name("bigEyes"), "Mắt to");
            assert_eq!(preset_name("profileBalance"), "Cân bằng nghiêng");
            assert_eq!(crate::i18n::tr(vi, "Face Reshape"), "Định hình khuôn mặt");
            assert_eq!(crate::i18n::tr(vi, "Face Reshape: Apply"), "Định hình khuôn mặt: Áp dụng");
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
        let rect = h.ctx.memory(|mem| mem.area_rect(panel_id())).expect("panel");
        assert!(rect.bottom() <= 900.0, "the panel stays inside 1440×900: {rect:?}");
        assert!(content.y > viewport.height(), "at 900 points the tall group scrolls instead of clipping: {content:?} in {viewport:?}");

        // With the room, the widest group fits without scrolling.
        let mut tall = draw(document_app(), vec2(1000.0, 1300.0), 0.25);
        tall.state_mut().ui.face_reshape.active_group = 0;
        tall.run_steps(3);
        let (viewport, content) = body_size(&tall);
        assert!(content.y <= viewport.height() + 1.0, "the tallest group fits when the window has room: {content:?} in {viewport:?}");

        let mut h = draw(document_app(), vec2(1000.0, 360.0), 0.25);
        h.state_mut().ui.face_reshape.active_group = 0;
        h.run_steps(3);
        let (viewport, content) = body_size(&h);
        assert!(content.y > viewport.height(), "a short window scrolls instead of clipping: {content:?} in {viewport:?}");
    }

    #[test]
    fn redesigned_panel_keeps_presets_faces_and_actions_visible_at_1440_by_900() {
        let mut h = draw(document_app(), vec2(1440.0, 900.0), 0.25);
        let rect = h.ctx.memory(|mem| mem.area_rect(panel_id())).expect("panel");
        assert!(rect.bottom() <= 900.0, "the panel stays inside the window: {rect:?}");
        for label in [
            entry_label(),
            subtitle(),
            preset_label(),
            face_mode_label(),
            face_main_label(),
            face_all_label(),
            face_selected_label(),
            compare_label(),
            reset_all_label(),
            reset_group_label(),
            apply_label(),
        ] {
            assert!(h.query_all_by_label(label).next().is_some(), "{label} is in the panel");
        }
        for index in 0..GROUPS.len() {
            let title = group_title(index);
            assert!(h.query_all_by_label(title).next().is_some(), "section {title} is in the panel");
        }
        // The sticky action area sits inside the window even though the sliders scroll.
        let apply = h.get_by_label(apply_label()).rect();
        assert!(rect.contains(apply.center()), "Apply stays visible: {apply:?} in {rect:?}");
        let (viewport, content) = body_size(&h);
        assert!(content.y > viewport.height(), "the 68 sliders scroll inside the body: {content:?} in {viewport:?}");

        // A status line grows the sticky card without pushing it out of the window.
        h.state_mut().ui.face_reshape.status = crate::i18n::fmt(preview_updated(), &[("points", "68")]);
        h.run_steps(3);
        let rect = h.ctx.memory(|mem| mem.area_rect(panel_id())).expect("panel");
        assert!(rect.bottom() <= 900.0, "the status line stays inside the window: {rect:?}");
        let apply = h.get_by_label(apply_label()).rect();
        assert!(rect.contains(apply.center()), "Apply stays visible with a status: {apply:?} in {rect:?}");
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

    #[test]
    fn builtin_presets_cover_the_six_named_looks() {
        assert_eq!(BUILTIN_PRESETS.len(), 6);
        assert_eq!(PRESET_NAMES.len(), 6);
        assert_eq!(preset_name("natural"), "Natural");
        assert_eq!(preset_name("vLine"), "V-Line");
        assert_eq!(preset_name("bigEyes"), "Big Eyes");
        assert_eq!(preset_name("slimNose"), "Slim Nose");
        assert_eq!(preset_name("fullLips"), "Full Lips");
        assert_eq!(preset_name("profileBalance"), "Profile Balance");
        for (key, values) in BUILTIN_PRESETS {
            assert!(!values.is_empty(), "{key} has values");
            let params = builtin_params(key).unwrap_or_else(|| panic!("{key} builds"));
            assert!(!params.is_identity(), "{key} changes something");
            for (slider, value) in values {
                assert_eq!(params.get(slider), Some(*value), "{key}.{slider}");
            }
        }
        assert!(builtin_params("missing").is_none());
    }

    #[test]
    fn user_presets_save_the_current_sixty_eight_sliders_and_round_trip() {
        let mut state = FaceReshapeState::default();
        state.params.set("jaw_width", 44.0);
        state.params.set("eye_size", -8.0);
        state.params.set("lip_plumpness", 12.0);
        state.save_preset("My Look").expect("a name saves");
        assert_eq!(state.last_preset, "user:My Look");
        assert_eq!(state.preset_label(), "My Look");
        let preset = state.presets.first().expect("saved");
        assert_eq!(preset.params.entries().len(), 68, "the full 68-slider set is captured");
        assert_eq!(preset.params.get("jaw_width"), Some(44.0));
        assert_eq!(preset.params.get("eye_size"), Some(-8.0));
        assert_eq!(preset.params.get("nose_scale"), Some(0.0), "untouched sliders save as zero");
        assert!(state.save_preset("  ").is_err(), "an empty name is refused");

        state.params = FaceReshapeParams::default();
        assert!(state.apply_user_preset("My Look"));
        assert_eq!(state.params.get("jaw_width"), Some(44.0));
        assert!(!state.apply_user_preset("missing"));
        assert_eq!(state.delete_selected_preset().as_deref(), Some("My Look"));
        assert!(state.presets.is_empty());

        state.save_preset("Saved").unwrap();
        let back: FaceReshapeState = serde_json::from_str(&serde_json::to_string(&state).expect("serialize")).expect("deserialize");
        assert_eq!(back.presets, state.presets, "user presets persist in UI state");
    }

    #[test]
    fn choosing_a_preset_sets_its_params_and_runs_a_preview() {
        let mut h = draw(document_app(), vec2(1000.0, 900.0), 0.25);
        h.query_all_by_role(egui::accesskit::Role::ComboBox).next().expect("the preset dropdown").click();
        h.run_steps(3);
        h.get_by_label("V-Line").click();
        h.run_steps(3);
        let state = &h.state().ui.face_reshape;
        assert_eq!(state.params.get("jaw_angle"), Some(18.0), "the preset set its sliders");
        assert_eq!(state.last_preset, "builtin:vLine");
        // The blank test document has no face, so the preview that the preset queued reports why.
        assert!(state.status_error, "{}", state.status);
        assert!(state.status.contains("no face"), "{}", state.status);
    }

    #[test]
    fn applying_a_builtin_sets_its_params_and_marks_the_panel_dirty() {
        let mut state = FaceReshapeState::default();
        assert!(state.apply_builtin("vLine"));
        assert_eq!(state.last_preset, "builtin:vLine");
        assert_eq!(state.preset_label(), "V-Line");
        assert_eq!(state.params.get("jaw_angle"), Some(18.0));
        assert_eq!(state.params.get("face_slim"), Some(25.0));
        assert!(state.dirty);
        assert!(!state.apply_builtin("missing"));
    }

    #[test]
    fn face_mode_and_selection_choose_the_engine_faces_value() {
        let mut state = FaceReshapeState::default();
        assert_eq!(state.faces_value(), json!("main"));
        state.face_mode = FaceMode::All;
        assert_eq!(state.faces_value(), json!("all"));
        state.set_detected_faces(3);
        state.face_mode = FaceMode::Selected;
        state.selected_faces.clear();
        assert_eq!(state.faces_value(), json!("main"), "an empty selection falls back to Main");
        state.selected_faces = vec![2, 3, 2];
        state.ensure();
        assert_eq!(state.faces_value(), json!([2, 3]));
        state.params.set("jaw_width", 20.0);
        assert_eq!(preview_payload(&state, None)["faces"], json!([2, 3]), "preview carries the selection");
        state.face_mode = FaceMode::All;
        assert_eq!(apply_payload(&state)["faces"], json!("all"), "apply carries the choice");
        state.face_mode = FaceMode::Selected;
        state.toggle_face(2, 3);
        assert_eq!(state.selected_faces, vec![3]);
        state.toggle_face(3, 3);
        assert_eq!(state.selected_faces, vec![3], "the last checked face stays");
        state.toggle_face(1, 3);
        assert_eq!(state.selected_faces, vec![1, 3]);
        state.set_detected_faces(1);
        assert_eq!(state.face_mode, FaceMode::Main);
        assert!(state.selected_faces.is_empty());
        assert_eq!(state.faces_value(), json!("main"));
    }

    #[test]
    fn checkboxes_appear_for_multiple_detected_faces_and_toggle_the_selection() {
        let mut h = draw(document_app(), vec2(1000.0, 900.0), 0.25);
        let state = &mut h.state_mut().ui.face_reshape;
        state.detected_faces = 3;
        state.face_mode = FaceMode::Selected;
        state.selected_faces = vec![1, 2, 3];
        h.run_steps(3);
        for n in 1..=3 {
            assert!(h.query_by_label(&format!("Face {n}")).is_some(), "face {n} checkbox");
        }
        h.get_by_label("Face 2").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.face_reshape.selected_faces, vec![1, 3], "clicking a box toggles that face");
    }

    #[test]
    fn face_mode_tabs_switch_and_selected_needs_two_faces() {
        let mut h = draw(document_app(), vec2(1000.0, 900.0), 0.25);
        h.get_by_label(face_all_label()).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.face_reshape.face_mode, FaceMode::All, "All is chosen from the tabs");
        h.get_by_label(face_selected_label()).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.face_reshape.face_mode, FaceMode::All, "Selected stays unavailable below two faces");
        h.state_mut().ui.face_reshape.detected_faces = 2;
        h.run_steps(2);
        h.get_by_label(face_selected_label()).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.face_reshape.face_mode, FaceMode::Selected, "two faces make Selected available");
        assert_eq!(h.state().ui.face_reshape.selected_faces, vec![1, 2], "both faces start checked");
        h.get_by_label(face_main_label()).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.face_reshape.face_mode, FaceMode::Main, "Main is chosen from the tabs");
    }

    #[test]
    fn menu_toggles_the_panel_and_apply_runs_the_panel_payload() {
        let ctx = egui::Context::default();
        let mut app = app();
        let value = crate::menus::invoke(&mut app, &ctx, "window.toggle.faceReshape", json!({})).expect("toggle");
        assert_eq!(value["open"], true);
        assert!(app.ui.panels.face_reshape);
        let value = crate::menus::invoke(&mut app, &ctx, "window.toggle.faceReshape", json!({})).expect("toggle");
        assert_eq!(value["open"], false);
        assert!(!app.ui.panels.face_reshape);
        assert!(menu(&mut app, "faceReshape.missing").is_none());

        // Apply opens the panel and runs the same payload the button sends. A blank document
        // has no face, so the panel reports the engine error rather than doing nothing.
        let mut app = document_app();
        app.ui.panels.face_reshape = false;
        app.ui.face_reshape.params.set("jaw_width", 20.0);
        let value = crate::menus::invoke(&mut app, &ctx, "faceReshape.apply", json!({})).expect("apply");
        assert_eq!(value["action"], "apply");
        assert!(app.ui.panels.face_reshape);
        assert!(app.ui.face_reshape.status_error, "{}", app.ui.face_reshape.status);
        assert!(app.ui.face_reshape.status.contains("no face"), "{}", app.ui.face_reshape.status);
    }

    #[test]
    fn shortcuts_are_listed_bound_and_conflict_free() {
        let app = app();
        let items = crate::prefs_ui::shortcut_items(&app);
        let table = crate::shortcut_dispatch::bindings(&app);
        for (id, key, path) in
            [("window.toggle.faceReshape", "Alt+Shift+M", &["Window"][..]), ("faceReshape.apply", "Face Reshape: Apply", &["Window", "Face Reshape"][..])]
        {
            if id == "faceReshape.apply" {
                assert_eq!(crate::shortcuts::default_shortcut(id).as_deref(), Some("Alt+Shift+D"), "{id} default");
            } else {
                assert_eq!(crate::shortcuts::default_shortcut(id).as_deref(), Some(key), "{id} default");
            }
            let item = items.iter().find(|item| item.0 == id).unwrap_or_else(|| panic!("{id} is listed"));
            assert_eq!(item.2, path.iter().map(|segment| (*segment).to_string()).collect::<Vec<_>>(), "{id} groups under its panel");
            let parsed =
                crate::shortcuts::parse(if id == "faceReshape.apply" { "Alt+Shift+D" } else { key }).unwrap_or_else(|| panic!("{id}: unparsable default"));
            assert!(table.iter().any(|(bound, sc)| bound == id && *sc == parsed), "{id} is bound");
        }
        // The unsafe keys stay with what already owns them.
        assert_eq!(crate::shortcuts::default_shortcut("basicColor.scanFaces").as_deref(), Some("Alt+Shift+F"));
        assert_ne!(crate::shortcuts::default_shortcut("window.toggle.faceReshape").as_deref(), Some("Alt+F"));
        let all = crate::menus::UI_COMMANDS
            .iter()
            .filter_map(|(id, _, _, sc)| sc.map(|s| (*id, s)))
            .chain(photocraft_engine::command_specs().iter().filter_map(|c| c.shortcut.map(|s| (c.id, s))));
        let conflicts = photocraft_engine::prefs::conflicts(all);
        assert!(conflicts.is_empty(), "the defaults conflict with nothing: {conflicts:?}");
    }

    /// Alt+Shift+M through the whole app: the shortcut opens the panel, the title is there and
    /// the six groups hold all 68 sliders. (Alt+F opens File, so the documented safe key is used.)
    #[test]
    fn shortcut_opens_the_panel_with_the_title_and_sixty_eight_sliders() {
        let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 128, "height": 128})).expect("new document");
            app
        });
        h.run_steps(8);
        assert!(!h.state().ui.panels.face_reshape);
        let sc = crate::shortcuts::parse("Alt+Shift+M").expect("parsable");
        let m = egui::Modifiers { alt: true, shift: true, ..Default::default() };
        h.event(egui::Event::ModifiersChanged(m));
        h.event(egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: true, repeat: false, modifiers: m });
        h.event(egui::Event::Key { key: sc.logical_key, physical_key: None, pressed: false, repeat: false, modifiers: m });
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(4);
        assert!(h.state().ui.panels.face_reshape, "the shortcut opens the window");
        assert!(h.query_all_by_label(entry_label()).next().is_some(), "the title is shown");
        let mut total = 0;
        for (index, keys) in GROUPS.iter().enumerate() {
            h.state_mut().ui.face_reshape.active_group = index;
            h.run_steps(3);
            let shown = h.query_all_by_role(egui::accesskit::Role::Slider).count();
            assert_eq!(shown, keys.len(), "group {index} shows its sliders");
            total += shown;
        }
        assert_eq!(total, 68, "all 68 sliders are reachable");
    }
}

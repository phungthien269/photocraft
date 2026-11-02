//! Face Reshape: non-destructive, parametric warping of the main face.
//!
//! `retouchAI.faceReshape` detects the best face in the source pixel layer, reads its 478
//! MediaPipe landmarks (via [`photocraft_algo::face_mesh`]), turns the 68 sliders into landmark
//! displacements, and warps only the face region through [`photocraft_algo::warp::warp_mesh_surface`].
//! The source layer is never touched: a preview holds the warped region above it, and Apply
//! renames that preview to the final layer.
//!
//! ## The 68 sliders
//!
//! The specification calls the set "70", but the supplied public list has 68 keys. Sliders are
//! `-100..=100`, default `0`, in the exact order below: head/global (17), eyes (13), eyebrows (7),
//! nose (11), mouth (12) and the finishing group (8). A positive value means "more" of the
//! slider's name (a wider jaw, a higher brow, thicker upper lip). Out-of-range numbers are
//! clamped; wrong types and unknown keys are errors.
//!
//! A few sliders describe depth, hair or teeth, which the 478 face landmarks cannot fully express:
//! `hair_volume`, `hairline_shift`, `cranium_height`, `chin_cleft`, `teeth_align` and the
//! projection sliders use the standard 2-D cues (local scale, a small shift of the feature toward
//! or away from the face centre, and the forehead/hairline contour) and are documented
//! approximations, not no-ops. The nose-profile sliders (`bridge_curvature`,
//! `nasal_hump_reduction`, `columella_height`, `radix_height`), `e_line_adjustment`,
//! `three_section_ratio`, `double_chin_warp` and `m_lip_shape` are also 2-D cues.
//!
//! ## Derived points
//!
//! MediaPipe's 478-point model includes the 10 iris points (5 per eye: centre plus
//! right/top/left/bottom) but no neck: 6 neck controls are derived below the chin from the face
//! box. They are internal controls only; the command still reports 478 landmarks.
//!
//! ## Field
//!
//! Every control point contributes its displacement with a compact, smooth kernel
//! (`(1 - d²/r²)³`) on a coarse grid over the face region. The grid is bilinearly sampled by the
//! warp map, and an edge fade brings the displacement to zero at the edge of the region, with
//! zero-displacement anchors on the perimeter. The warp runs only on the face rectangle, so a
//! 24 MP document never copies or resamples the whole canvas.
//!
//! Detection runs on a reduced, source-only composite (long side 1600 px), then the landmark
//! model runs on a full-resolution crop around each selected box. `faces` picks the main face
//! (the default), every detected face, or a set of 1-based faces in the detector's order; all of
//! them are warped into the same preview/final layer. On the browser build neither model exists,
//! so the command is disabled with the same message as `retouchAI.faceLandmarks`.

use std::sync::OnceLock;

use photocraft_algo::face_mesh;
use photocraft_algo::faces;
use photocraft_algo::transform::Interp;
use photocraft_algo::warp::warp_mesh_surface;
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Command id.
pub const CMD: &str = "retouchAI.faceReshape";
/// Exact name of the live preview layer.
pub const PREVIEW_NAME: &str = "Face Reshape Preview";
/// Exact name of the applied layer.
pub const FINAL_NAME: &str = "Face Reshape";

const ANALYSIS_SIDE: u32 = 1600;
const MIN_SCORE: f32 = crate::face_cmds::MIN_SCORE;
/// How far the rotation/translation region extends past the detector box (fractions of its size).
const REGION_LEFT: f32 = 0.35;
const REGION_TOP: f32 = 0.35;
const REGION_RIGHT: f32 = 0.35;
/// More room below the chin for the neck controls.
const REGION_BOTTOM: f32 = 0.55;
/// Kernel radius as a fraction of the face's longest side.
const KERNEL: f32 = 0.28;
/// Fade width at the region edge as a fraction of the region's shorter side.
const EDGE_FADE: f32 = 0.14;
const GRID_MIN: usize = 24;
const GRID_MAX: usize = 128;
const GRID_PIXELS_PER_CELL: u32 = 6;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

// ---------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------

/// Declares the typed slider set, its JSON keys, and the three accessors the command and tests
/// need. Keeping the fields next to the keys in one list is what makes "all 68 keys" auditable.
macro_rules! face_reshape_params {
    ($($field:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default)]
        pub struct FaceReshapeParams {
            $(pub $field: f32,)*
        }

        impl FaceReshapeParams {
            /// Every slider key, in the documented order (`head_scale` … `midface_projection`).
            pub const KEYS: &'static [&'static str] = &[$(stringify!($field)),*];
            /// Number of sliders (68).
            pub const COUNT: usize = Self::KEYS.len();

            /// A slider by name.
            pub fn get(&self, key: &str) -> Option<f32> {
                match key {
                    $(stringify!($field) => Some(self.$field),)*
                    _ => None,
                }
            }

            fn slot_mut(&mut self, key: &str) -> Option<&mut f32> {
                match key {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }

            /// `(key, value)` for every slider, in [`Self::KEYS`] order.
            fn entries(&self) -> [(&'static str, f32); Self::COUNT] {
                [$( (stringify!($field), self.$field) ),*]
            }

            /// True when every slider is at 0, so the warp is the identity.
            pub fn is_identity(&self) -> bool {
                $(self.$field == 0.0)&&*
            }

            /// Parse the `params` object: missing keys are 0, numbers outside `-100..=100` are
            /// clamped, and a wrong type, a non-finite number or an unknown key is an error.
            pub fn from_value(value: &Value) -> std::result::Result<Self, String> {
                let object = value.as_object().ok_or("`params` must be an object of face sliders")?;
                let mut out = Self::default();
                for (key, value) in object {
                    let Some(slot) = out.slot_mut(key) else {
                        return Err(format!("unknown face reshape slider `{key}`"));
                    };
                    let number = value
                        .as_f64()
                        .ok_or_else(|| format!("`{key}` must be a number in -100..=100"))?;
                    if !number.is_finite() {
                        return Err(format!("`{key}` must be finite"));
                    }
                    *slot = (number as f32).clamp(-100.0, 100.0);
                }
                Ok(out)
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
    // Finishing: proportions, symmetry, neck and depth.
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
// MediaPipe topology
// ---------------------------------------------------------------------------------------------

const FOREHEAD: &[usize] =
    &[10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 9, 8, 168, 6, 197, 195, 5, 4, 1, 2, 98, 327];
const TEMPLE: &[usize] = &[127, 234, 93, 132, 58, 172, 136, 150, 149, 176, 148, 377, 400, 378, 379, 365, 397, 288, 361, 323, 454, 356];
/// The top arc of the face oval: the forehead/hairline cue (the model has no hair points).
const HAIRLINE: &[usize] = &[10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377];
const JAW: &[usize] =
    &[58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109, 10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377];
const CHIN: &[usize] = &[152, 148, 176, 149, 150, 136, 172, 377, 400, 378, 379, 365, 397, 288, 361, 323, 454, 356];

const EYE_A: &[usize] = &[33, 7, 163, 144, 145, 153, 154, 155, 133];
const EYE_B: &[usize] = &[263, 249, 390, 373, 374, 380, 381, 382, 362];
const EYE_A_OUTER: usize = 33;
const EYE_A_INNER: usize = 133;
const EYE_B_OUTER: usize = 263;
const EYE_B_INNER: usize = 362;
/// Iris controls in the 478-point model: [`EYE_A`] then [`EYE_B`], five points each.
const IRIS_A: &[usize] = &[468, 469, 470, 471, 472];
const IRIS_B: &[usize] = &[473, 474, 475, 476, 477];
/// Both eyes and their irises: symmetry must pair the left eye with the right one, so the
/// group has to contain both sides.
const EYES_ALL: &[usize] =
    &[33, 7, 163, 144, 145, 153, 154, 155, 133, 263, 249, 390, 373, 374, 380, 381, 382, 362, 468, 469, 470, 471, 472, 473, 474, 475, 476, 477];

const BROW_A: &[usize] = &[46, 53, 52, 65, 55, 70, 63, 105, 66, 107];
const BROW_B: &[usize] = &[276, 283, 282, 295, 285, 300, 293, 334, 296, 336];
/// Both eyebrows, for the same reason as [`EYES_ALL`].
const BROWS: &[usize] = &[46, 53, 52, 65, 55, 70, 63, 105, 66, 107, 276, 283, 282, 295, 285, 300, 293, 334, 296, 336];

const NOSE_BRIDGE: &[usize] =
    &[168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 326, 294, 278, 344, 440, 275, 45, 220, 115, 48, 64, 219, 439, 218, 438, 237, 457, 43, 273, 46, 276, 44, 274];
const NOSE_TIP: &[usize] = &[
    1, 2, 98, 327, 168, 6, 195, 197, 5, 4, 45, 220, 115, 48, 64, 275, 440, 344, 278, 294, 331, 102, 129, 358, 357, 350, 349, 277, 279, 429, 437, 399, 419, 456,
];
const NOSTRIL: &[usize] = &[102, 331, 98, 327, 294, 278, 344, 440, 275, 45, 220, 115, 48, 64, 129, 358, 357, 350, 349, 277, 279, 429, 437, 399, 419, 456];
const NOSE_ROOT: &[usize] = &[168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 326, 294, 278, 344, 440, 275, 45, 220];
/// The middle of the nose bridge: the 2-D cue for curvature and a dorsal hump.
const BRIDGE_MID: &[usize] = &[168, 6, 197, 195, 5, 4, 1, 2, 98, 327];
/// The base of the nose between the nostrils: the columella cue.
const COLUMELLA: &[usize] = &[1, 2, 98, 327, 129, 358, 357, 350, 349, 277, 279, 429, 437, 399, 419, 456];
const NOSE_ALL: &[usize] = &[
    168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 326, 294, 278, 344, 440, 275, 45, 220, 115, 48, 64, 219, 439, 218, 438, 237, 457, 43, 273, 46, 276, 44, 274, 331,
    102, 129, 358, 357, 350, 349, 277, 279, 429, 437, 399, 419, 456, 248, 281, 363, 434, 364, 367, 435, 397, 288, 361, 323, 454, 356,
];

const CHEEK_A: &[usize] =
    &[50, 101, 36, 142, 187, 205, 206, 207, 216, 192, 213, 147, 123, 117, 118, 119, 120, 121, 128, 245, 188, 116, 111, 31, 228, 229, 230, 231, 232, 233];
const CHEEK_B: &[usize] =
    &[280, 330, 266, 371, 411, 425, 426, 427, 436, 416, 433, 376, 352, 346, 347, 348, 349, 350, 357, 465, 412, 345, 340, 261, 448, 449, 450, 451, 452, 453];
const MIDFACE: &[usize] = &[
    50, 101, 36, 142, 187, 205, 206, 207, 216, 192, 213, 147, 123, 117, 118, 119, 120, 121, 128, 245, 188, 116, 111, 31, 228, 229, 230, 231, 232, 233, 280,
    330, 266, 371, 411, 425, 426, 427, 436, 416, 433, 376, 352, 346, 347, 348, 349, 350, 357, 465, 412, 345, 340, 261, 448, 449, 450, 451, 452, 453, 1, 2, 4,
    5, 6, 168, 195, 197, 98, 327, 3, 51, 281, 48, 278, 219, 439, 218, 438, 237, 457, 43, 273, 46, 276, 44, 274, 45, 275,
];

const UPPER_LIP: &[usize] = &[61, 185, 40, 39, 37, 0, 267, 269, 270, 409];
const LOWER_LIP: &[usize] = &[61, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291];
const INNER_UPPER_LIP: &[usize] = &[78, 191, 80, 81, 82, 13, 312, 311, 310, 415];
const INNER_LOWER_LIP: &[usize] = &[308, 324, 318, 402, 317, 14, 87, 178, 88, 95];
const MOUTH: &[usize] = &[
    61, 185, 40, 39, 37, 0, 267, 269, 270, 409, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291, 78, 191, 80, 81, 82, 13, 312, 311, 310, 415, 308, 324, 318, 402,
    317, 14, 87, 178, 88, 95,
];
const MOUTH_CORNERS: &[usize] = &[61, 291];
const PHILTRUM: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 19, 94, 98, 168, 195, 197, 327, 326];

/// Derived neck controls, below the chin (see [`Geometry::from_landmarks`]). They follow the
/// 478 model points, so their indices start at 478.
const NECK: &[usize] = &[478, 479, 480, 481, 482, 483];
const ALL_FACE: &[usize] = &[
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
    41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79,
    80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114,
    115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139, 140, 141, 142, 143, 144, 145,
    146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176,
    177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207,
    208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 237, 238,
    239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 251, 252, 253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 267, 268, 269,
    270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283, 284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300,
    301, 302, 303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314, 315, 316, 317, 318, 319, 320, 321, 322, 323, 324, 325, 326, 327, 328, 329, 330, 331,
    332, 333, 334, 335, 336, 337, 338, 339, 340, 341, 342, 343, 344, 345, 346, 347, 348, 349, 350, 351, 352, 353, 354, 355, 356, 357, 358, 359, 360, 361, 362,
    363, 364, 365, 366, 367, 368, 369, 370, 371, 372, 373, 374, 375, 376, 377, 378, 379, 380, 381, 382, 383, 384, 385, 386, 387, 388, 389, 390, 391, 392, 393,
    394, 395, 396, 397, 398, 399, 400, 401, 402, 403, 404, 405, 406, 407, 408, 409, 410, 411, 412, 413, 414, 415, 416, 417, 418, 419, 420, 421, 422, 423, 424,
    425, 426, 427, 428, 429, 430, 431, 432, 433, 434, 435, 436, 437, 438, 439, 440, 441, 442, 443, 444, 445, 446, 447, 448, 449, 450, 451, 452, 453, 454, 455,
    456, 457, 458, 459, 460, 461, 462, 463, 464, 465, 466, 467, 468, 469, 470, 471, 472, 473, 474, 475, 476, 477,
];

// ---------------------------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------------------------

/// Landmark geometry for one face: the 478 model points (including the 10 iris controls) and 6
/// derived neck controls, plus the face box and a mirror-pair map.
struct Geometry {
    base: Vec<[f64; 2]>,
    pts: Vec<[f64; 2]>,
    face: Rect,
    center: [f64; 2],
    size: f64,
    mirror: Vec<(usize, usize)>,
}

impl Geometry {
    fn from_landmarks(points: &[[f32; 3]], face: Rect) -> std::result::Result<Self, String> {
        if points.len() != face_mesh::POINTS {
            return Err(format!("face reshape needs {} landmarks, got {}", face_mesh::POINTS, points.len()));
        }
        if face.is_empty() {
            return Err("face reshape needs a face box with a positive size".into());
        }
        let mut base = Vec::with_capacity(points.len() + 6);
        for point in points {
            if !point[0].is_finite() || !point[1].is_finite() {
                return Err("face reshape got a non-finite landmark".into());
            }
            base.push([f64::from(point[0]), f64::from(point[1])]);
        }
        let center = [f64::from(face.x0) + f64::from(face.width()) * 0.5, f64::from(face.y0) + f64::from(face.height()) * 0.5];
        let size = f64::from(face.width().max(face.height())).max(1.0);

        // 6 derived neck controls below the chin; the 478 model covers the face and eyes only.
        let chin = base.get(152).copied().unwrap_or([center[0], f64::from(face.y1)]);
        let w = f64::from(face.width());
        let h = f64::from(face.height());
        for (dx, dy) in [(-0.18, 0.16), (0.18, 0.16), (-0.30, 0.35), (0.30, 0.35), (0.0, 0.42), (0.0, 0.24)] {
            base.push([center[0] + w * dx, chin[1] + h * dy]);
        }

        let mirror = mirror_pairs(&base, center[0], size * 0.04);
        Ok(Self { pts: base.clone(), base, face, center, size, mirror })
    }

    /// Apply every non-zero slider in the documented order.
    fn apply(&mut self, params: &FaceReshapeParams) {
        for (key, value) in params.entries() {
            if value != 0.0 {
                self.apply_one(key, f64::from(value) / 100.0);
            }
        }
    }

    /// Apply one slider at `t` in `-1..=1`. Each arm moves a real landmark group; the helpers
    /// below keep the direction conventions consistent (positive = more of the slider's name).
    /// The few sliders that describe depth, hair or teeth are documented 2-D approximations.
    fn apply_one(&mut self, key: &str, t: f64) {
        let d = t * self.size;
        match key {
            "head_scale" => {
                let s = 1.0 + t * 0.06;
                self.scale_about(all_face_plus_derived(), self.center, s, s);
            }
            "cranium_height" => {
                // Stretch the upper head about the eye line: the closest 2-D cue for a taller
                // cranium when the model only has the face oval.
                let pivot = [self.center[0], f64::from(self.face.y0) + f64::from(self.face.height()) * 0.55];
                let s = 1.0 + t * 0.06;
                self.scale_groups_about(&[FOREHEAD, HAIRLINE], pivot, 1.0, s);
                self.translate(HAIRLINE, 0.0, -d * 0.005);
            }
            "hair_volume" => {
                let pivot = [self.center[0], f64::from(self.face.y0) + f64::from(self.face.height()) * 0.45];
                let sx = 1.0 + t * 0.05;
                let sy = 1.0 + t * 0.07;
                self.scale_groups_about(&[FOREHEAD, HAIRLINE], pivot, sx, sy);
            }
            "hairline_shift" => self.translate(HAIRLINE, 0.0, -d * 0.03),
            "face_slim" => {
                let s = 1.0 - t * 0.05;
                self.scale_groups_about(&[CHEEK_A, CHEEK_B, JAW, CHIN], self.center, s, 1.0);
            }
            "face_width" => {
                let s = 1.0 + t * 0.06;
                self.scale_groups_about(&[CHEEK_A, CHEEK_B, JAW, CHIN, MIDFACE], self.center, s, 1.0);
            }
            "face_length" => {
                let s = 1.0 + t * 0.06;
                self.scale_groups_about(&[CHEEK_A, CHEEK_B, JAW, CHIN, MIDFACE], self.center, 1.0, s);
            }
            "jaw_width" => self.scale_about(JAW, self.center, 1.0 + t * 0.06, 1.0),
            "jaw_angle" => {
                // Sharpen the angle: pull the lower jaw sides up and in.
                let half = (f64::from(self.face.height()) * 0.5).max(1.0);
                for &i in JAW {
                    if let Some(p) = self.pts.get_mut(i) {
                        let below = ((p[1] - self.center[1]) / half).clamp(0.0, 1.0);
                        let side = p[0] - self.center[0];
                        if side.abs() > 1e-6 {
                            p[0] -= side.signum() * t * 0.012 * self.size * below;
                        }
                        p[1] -= t * 0.012 * self.size * below;
                    }
                }
            }
            "cheekbone_shrink" => {
                let s = 1.0 - t * 0.05;
                self.scale_groups_about(&[CHEEK_A, CHEEK_B], self.center, s, 1.0);
            }
            "cheek_slim" => {
                self.push_side(CHEEK_A, -t * 0.012);
                self.push_side(CHEEK_B, -t * 0.012);
                self.translate(CHEEK_A, 0.0, d * 0.004);
                self.translate(CHEEK_B, 0.0, d * 0.004);
            }
            "temple_fill" => {
                self.push_side(TEMPLE, t * 0.012);
                self.scale_about(TEMPLE, self.center, 1.0 + t * 0.03, 1.0);
            }
            "chin_length" => self.translate(CHIN, 0.0, d * 0.02),
            "chin_width" => self.scale_group_about_own_center(CHIN, 1.0 + t * 0.10, 1.0),
            "chin_protrusion" => {
                // Approximated in 2-D with a slightly larger, lower chin.
                self.scale_group_about_own_center(CHIN, 1.0 + t * 0.06, 1.0 + t * 0.04);
                self.translate(CHIN, 0.0, d * 0.006);
            }
            "chin_cleft" => {
                // A cleft is a groove in the chin; the 2-D cue is lifting its midpoint and
                // widening the two sides a touch.
                self.translate(&[152], 0.0, -d * 0.006);
                self.push_side(&[148, 176, 377, 400], t * 0.003);
            }
            "double_chin_warp" => {
                // Lift the submental tissue (the derived neck controls) and keep the chin edge.
                self.translate(&[478, 479, 483], 0.0, -d * 0.025);
                self.translate(&[480, 481, 482], 0.0, -d * 0.012);
                self.translate(&[152, 148, 176, 377, 400], 0.0, d * 0.004);
            }

            "eye_size" => {
                let s = 1.0 + t * 0.10;
                self.scale_eyes(s, s);
            }
            "eye_width" => {
                let s = 1.0 + t * 0.12;
                self.scale_eyes(s, 1.0);
            }
            "eye_height" => {
                let s = 1.0 + t * 0.12;
                self.scale_eyes(1.0, s);
            }
            "eye_distance" => {
                self.translate(EYE_A, -d * 0.02, 0.0);
                self.translate(IRIS_A, -d * 0.02, 0.0);
                self.translate(EYE_B, d * 0.02, 0.0);
                self.translate(IRIS_B, d * 0.02, 0.0);
            }
            "eye_tilt" => {
                self.tilt_eye(EYE_A, IRIS_A, EYE_A_OUTER, EYE_A_INNER, t);
                self.tilt_eye(EYE_B, IRIS_B, EYE_B_OUTER, EYE_B_INNER, t);
            }
            "eye_position_y" => {
                self.translate(EYE_A, 0.0, -d * 0.015);
                self.translate(EYE_B, 0.0, -d * 0.015);
                self.translate(IRIS_A, 0.0, -d * 0.015);
                self.translate(IRIS_B, 0.0, -d * 0.015);
            }
            "inner_canthus" => {
                self.translate(&[EYE_A_INNER], -d * 0.02, -d * 0.005);
                self.translate(&[EYE_B_INNER], d * 0.02, -d * 0.005);
            }
            "outer_canthus" => {
                self.translate(&[EYE_A_OUTER], -d * 0.02, -d * 0.006);
                self.translate(&[EYE_B_OUTER], d * 0.02, -d * 0.006);
            }
            "lower_eyelid_down" => {
                self.move_lid(EYE_A, t, true);
                self.move_lid(EYE_B, t, true);
            }
            "pupil_scale" => {
                for (eye, iris) in [(EYE_A, IRIS_A), (EYE_B, IRIS_B)] {
                    let c = centroid_of(&self.pts, eye);
                    self.scale_about(iris, c, 1.0 + t * 0.25, 1.0 + t * 0.25);
                }
            }
            "aegyo_sal" => {
                // Under-eye fullness: the lower lid drops slightly and the upper cheek lifts.
                self.move_lid(EYE_A, t * 0.35, true);
                self.move_lid(EYE_B, t * 0.35, true);
                self.translate(CHEEK_A, 0.0, -d * 0.006);
                self.translate(CHEEK_B, 0.0, -d * 0.006);
            }
            "eye_brow_distance" => {
                self.translate(BROW_A, 0.0, -d * 0.015);
                self.translate(BROW_B, 0.0, -d * 0.015);
            }
            "eyelid_fold_width" => {
                self.move_lid(EYE_A, t, false);
                self.move_lid(EYE_B, t, false);
            }

            "eyebrow_height" => {
                self.translate(BROW_A, 0.0, -d * 0.02);
                self.translate(BROW_B, 0.0, -d * 0.02);
            }
            "eyebrow_distance" => {
                self.translate(BROW_A, -d * 0.015, 0.0);
                self.translate(BROW_B, d * 0.015, 0.0);
            }
            "eyebrow_length" => {
                self.scale_group_about_own_center(BROW_A, 1.0 + t * 0.08, 1.0);
                self.scale_group_about_own_center(BROW_B, 1.0 + t * 0.08, 1.0);
            }
            "eyebrow_thickness" => {
                self.scale_group_about_own_center(BROW_A, 1.0, 1.0 + t * 0.18);
                self.scale_group_about_own_center(BROW_B, 1.0, 1.0 + t * 0.18);
            }
            "eyebrow_arch" => {
                self.arch_brow(BROW_A, EYE_A_INNER, EYE_A_OUTER, t);
                self.arch_brow(BROW_B, EYE_B_INNER, EYE_B_OUTER, t);
            }
            "eyebrow_tail" => {
                self.brow_tail(&[66, 107], t);
                self.brow_tail(&[296, 336], t);
            }
            "eyebrow_tilt" => {
                self.brow_tilt(BROW_A, EYE_A_INNER, EYE_A_OUTER, t);
                self.brow_tilt(BROW_B, EYE_B_INNER, EYE_B_OUTER, t);
            }

            "nose_scale" => {
                let c = centroid_of(&self.pts, NOSE_ALL);
                let s = 1.0 + t * 0.06;
                self.scale_about(NOSE_ALL, c, s, s);
            }
            "nose_wing_width" => {
                let c = centroid_of(&self.pts, NOSTRIL);
                self.scale_about(NOSTRIL, c, 1.0 + t * 0.10, 1.0);
            }
            "nose_tip_size" => {
                let c = centroid_of(&self.pts, NOSE_TIP);
                let s = 1.0 + t * 0.12;
                self.scale_about(NOSE_TIP, c, s, s);
            }
            "nose_tip_angle" => {
                let c = centroid_of(&self.pts, NOSE_TIP);
                for &i in NOSE_TIP {
                    if let Some(p) = self.pts.get_mut(i) {
                        p[1] -= t * 0.14 * (p[0] - c[0]);
                    }
                }
            }
            "nose_length" => {
                self.translate(NOSE_TIP, 0.0, d * 0.02);
                self.translate(NOSTRIL, 0.0, d * 0.02);
                self.translate(NOSE_BRIDGE, 0.0, d * 0.008);
            }
            "bridge_width" => self.scale_group_about_own_center(NOSE_BRIDGE, 1.0 + t * 0.08, 1.0),
            "bridge_curvature" => {
                let c = centroid_of(&self.pts, BRIDGE_MID);
                self.scale_about(BRIDGE_MID, c, 1.0 + t * 0.05, 1.0);
                self.translate(BRIDGE_MID, 0.0, d * 0.004);
            }
            "nasal_hump_reduction" => {
                let c = centroid_of(&self.pts, BRIDGE_MID);
                self.scale_about(BRIDGE_MID, c, 1.0 - t * 0.04, 1.0);
                self.translate(BRIDGE_MID, 0.0, -d * 0.006);
            }
            "columella_height" => self.translate(COLUMELLA, 0.0, -d * 0.008),
            "radix_height" => self.translate(&[168, 6, 197, 195, 5, 4], 0.0, -d * 0.012),
            "nostril_size" => {
                let c = centroid_of(&self.pts, NOSTRIL);
                self.scale_about(NOSTRIL, c, 1.0 + t * 0.10, 1.0 + t * 0.10);
            }

            "mouth_scale" => {
                let c = centroid_of(&self.pts, MOUTH);
                let s = 1.0 + t * 0.05;
                self.scale_about(MOUTH, c, s, s);
            }
            "mouth_width" => self.scale_group_about_own_center(MOUTH, 1.0 + t * 0.10, 1.0),
            "mouth_position_y" => self.translate(MOUTH, 0.0, d * 0.02),
            "philtrum_length" => {
                self.translate(UPPER_LIP, 0.0, d * 0.012);
                self.translate(INNER_UPPER_LIP, 0.0, d * 0.012);
                self.translate(PHILTRUM, 0.0, -d * 0.004);
            }
            "upper_lip_thickness" => {
                self.translate(UPPER_LIP, 0.0, -d * 0.012);
                self.translate(INNER_UPPER_LIP, 0.0, -d * 0.006);
            }
            "lower_lip_thickness" => {
                self.translate(LOWER_LIP, 0.0, d * 0.012);
                self.translate(INNER_LOWER_LIP, 0.0, d * 0.006);
            }
            "smile_corners" => {
                self.translate(MOUTH_CORNERS, 0.0, -d * 0.02);
                self.translate(&[185, 40, 409, 270], 0.0, -d * 0.01);
            }
            "m_lip_shape" => self.m_lip(t),
            "lip_plumpness" => {
                let c = centroid_of(&self.pts, MOUTH);
                self.scale_about(MOUTH, c, 1.0 + t * 0.04, 1.0 + t * 0.04);
                self.translate(UPPER_LIP, 0.0, -d * 0.004);
                self.translate(LOWER_LIP, 0.0, d * 0.004);
            }
            "lip_protrusion" => {
                // Approximated with a pout: the lips part vertically and widen a touch.
                self.translate(UPPER_LIP, 0.0, -d * 0.008);
                self.translate(LOWER_LIP, 0.0, d * 0.008);
                self.scale_group_about_own_center(MOUTH, 1.0 + t * 0.02, 1.0);
            }
            "lower_lip_to_chin" => {
                self.translate(LOWER_LIP, 0.0, -d * 0.006);
                self.translate(CHIN, 0.0, d * 0.004);
            }
            "teeth_align" => {
                // The model has no teeth: open the inner mouth a touch and widen it, the 2-D cue
                // for showing a straighter row.
                self.translate(INNER_UPPER_LIP, 0.0, -d * 0.004);
                self.translate(INNER_LOWER_LIP, 0.0, d * 0.004);
                let c = centroid_of(&self.pts, MOUTH);
                self.scale_about(INNER_UPPER_LIP, c, 1.0 + t * 0.02, 1.0);
                self.scale_about(INNER_LOWER_LIP, c, 1.0 + t * 0.02, 1.0);
            }

            "three_section_ratio" => {
                self.translate(FOREHEAD, 0.0, -d * 0.008);
                self.translate(BROW_A, 0.0, -d * 0.004);
                self.translate(BROW_B, 0.0, -d * 0.004);
                self.translate(NOSE_ROOT, 0.0, d * 0.004);
                self.translate(NOSE_TIP, 0.0, d * 0.004);
            }
            "face_symmetry" => self.symmetrize_group(ALL_FACE, t),
            "eye_symmetry" => self.symmetrize_group(EYES_ALL, t),
            "eyebrow_symmetry" => self.symmetrize_group(BROWS, t),
            "neck_slim" => {
                let c = centroid_of(&self.pts, NECK);
                self.scale_about(NECK, c, 1.0 - t * 0.12, 1.0);
            }
            "neck_length" => self.translate(NECK, 0.0, d * 0.03),
            "e_line_adjustment" => {
                // The aesthetic E-line runs from the nose tip to the chin; move the chin forward
                // (down, in this 2-D projection) and the tip back a little.
                self.translate(CHIN, 0.0, d * 0.008);
                self.translate(NOSE_TIP, 0.0, -d * 0.003);
            }
            "midface_projection" => {
                // Approximated with a small outward push and scale of the cheek/nose-root band.
                self.scale_about(MIDFACE, self.center, 1.0 + t * 0.03, 1.0);
                self.push_side(MIDFACE, t * 0.010);
            }
            _ => {}
        }
    }

    /// Move the lower (`lower = true`) or upper half of an eye contour vertically.
    fn move_lid(&mut self, eye: &[usize], t: f64, lower: bool) {
        let c = centroid_of(&self.pts, eye);
        let dy = if lower { t * self.size * 0.008 } else { -t * self.size * 0.008 };
        for &i in eye {
            if let Some(p) = self.pts.get_mut(i) {
                let on_lower = p[1] >= c[1];
                if on_lower == lower {
                    p[1] += dy;
                }
            }
        }
    }

    /// Lift and lengthen the outer tail of an eyebrow.
    fn brow_tail(&mut self, tail: &[usize], t: f64) {
        let d = t * self.size * 0.012;
        for &i in tail {
            if let Some(p) = self.pts.get_mut(i) {
                let side = p[0] - self.center[0];
                if side.abs() > 1e-6 {
                    p[0] += side.signum() * d;
                }
                p[1] -= d;
            }
        }
    }

    /// Tilt an eyebrow linearly from its inner end to its outer end.
    fn brow_tilt(&mut self, brow: &[usize], inner: usize, outer: usize, t: f64) {
        let inner_x = self.pts.get(inner).map_or(0.0, |p| p[0]);
        let outer_x = self.pts.get(outer).map_or(0.0, |p| p[0]);
        let span = outer_x - inner_x;
        if span.abs() < 1e-9 {
            return;
        }
        for &i in brow {
            if let Some(p) = self.pts.get_mut(i) {
                let nx = ((p[0] - inner_x) / span).clamp(0.0, 1.0);
                p[1] -= t * self.size * 0.015 * nx;
            }
        }
    }

    /// Cupid's bow: lift the two upper-lip peaks and dip the centre.
    fn m_lip(&mut self, t: f64) {
        let d = t * self.size;
        self.translate(&[37, 267], 0.0, -d * 0.006);
        self.translate(&[0], 0.0, d * 0.004);
        self.translate(&[39, 269], 0.0, -d * 0.002);
    }

    fn translate(&mut self, group: &[usize], dx: f64, dy: f64) {
        for &i in group {
            if let Some(p) = self.pts.get_mut(i) {
                p[0] += dx;
                p[1] += dy;
            }
        }
    }

    fn scale_about(&mut self, group: &[usize], pivot: [f64; 2], sx: f64, sy: f64) {
        for &i in group {
            if let Some(p) = self.pts.get_mut(i) {
                p[0] = pivot[0] + (p[0] - pivot[0]) * sx;
                p[1] = pivot[1] + (p[1] - pivot[1]) * sy;
            }
        }
    }

    /// Scale several groups once each, even where they overlap (the topology lists share
    /// contour, cheek and lip points; applying a scale twice would square its effect).
    fn scale_groups_about(&mut self, groups: &[&[usize]], pivot: [f64; 2], sx: f64, sy: f64) {
        let mut seen = vec![false; self.pts.len()];
        for group in groups {
            for &i in *group {
                if seen.get(i) == Some(&true) {
                    continue;
                }
                if let Some(slot) = seen.get_mut(i) {
                    *slot = true;
                }
                if let Some(p) = self.pts.get_mut(i) {
                    p[0] = pivot[0] + (p[0] - pivot[0]) * sx;
                    p[1] = pivot[1] + (p[1] - pivot[1]) * sy;
                }
            }
        }
    }

    fn scale_group_about_own_center(&mut self, group: &[usize], sx: f64, sy: f64) {
        let c = centroid_of(&self.pts, group);
        self.scale_groups_about(&[group], c, sx, sy);
    }

    fn scale_eyes(&mut self, sx: f64, sy: f64) {
        for (eye, iris) in [(EYE_A, IRIS_A), (EYE_B, IRIS_B)] {
            let c = centroid_of(&self.pts, eye);
            self.scale_about(eye, c, sx, sy);
            self.scale_about(iris, c, sx, sy);
        }
    }

    /// Move the outer corner up and the inner corner down, with a linear gradient across the eye.
    fn tilt_eye(&mut self, eye: &[usize], iris: &[usize], outer: usize, inner: usize, t: f64) {
        let outer_x = self.pts.get(outer).map_or(0.0, |p| p[0]);
        let inner_x = self.pts.get(inner).map_or(0.0, |p| p[0]);
        let span = inner_x - outer_x;
        if span.abs() < 1e-9 {
            return;
        }
        for &i in eye.iter().chain(iris) {
            if let Some(p) = self.pts.get_mut(i) {
                let nx = ((p[0] - outer_x) / span).clamp(0.0, 1.0);
                p[1] -= t * self.size * 0.018 * (1.0 - nx);
            }
        }
    }

    fn arch_brow(&mut self, brow: &[usize], inner: usize, outer: usize, t: f64) {
        let inner_x = self.pts.get(inner).map_or(0.0, |p| p[0]);
        let outer_x = self.pts.get(outer).map_or(0.0, |p| p[0]);
        let span = outer_x - inner_x;
        if span.abs() < 1e-9 {
            return;
        }
        for &i in brow {
            if let Some(p) = self.pts.get_mut(i) {
                let nx = ((p[0] - inner_x) / span).clamp(0.0, 1.0);
                let lift = (std::f64::consts::PI * nx).sin();
                p[1] -= t * self.size * 0.02 * lift;
            }
        }
    }

    /// Push each point away from the vertical centre line by `amount` (negative pulls in).
    fn push_side(&mut self, group: &[usize], amount: f64) {
        for &i in group {
            if let Some(p) = self.pts.get_mut(i) {
                let side = p[0] - self.center[0];
                if side.abs() > 1e-6 {
                    p[0] += side.signum() * amount;
                }
            }
        }
    }

    fn symmetrize_group(&mut self, group: &[usize], t: f64) {
        if t == 0.0 {
            return;
        }
        let mut members = vec![false; self.pts.len()];
        for &i in group {
            if let Some(m) = members.get_mut(i) {
                *m = true;
            }
        }
        let mut pairs: Vec<(usize, usize)> =
            self.mirror.iter().copied().filter(|&(a, b)| members.get(a) == Some(&true) && members.get(b) == Some(&true)).collect();
        if pairs.is_empty() {
            // The automatic map is tolerance-based; a thin group can still be paired by taking
            // the closest mirrored partner inside the group.
            let mut used = vec![false; self.pts.len()];
            for (k, &i) in group.iter().enumerate() {
                if used.get(i) == Some(&true) {
                    continue;
                }
                let Some(pi) = self.pts.get(i).copied() else { continue };
                let target = [2.0 * self.center[0] - pi[0], pi[1]];
                let mut best: Option<(usize, f64)> = None;
                for &j in group.iter().skip(k + 1) {
                    if used.get(j) == Some(&true) {
                        continue;
                    }
                    let Some(pj) = self.pts.get(j) else { continue };
                    let dist = (pj[0] - target[0]).hypot(pj[1] - target[1]);
                    if best.is_none_or(|(_, d)| dist < d) {
                        best = Some((j, dist));
                    }
                }
                if let Some((j, dist)) = best
                    && dist <= self.size * 0.20
                {
                    if let Some(u) = used.get_mut(i) {
                        *u = true;
                    }
                    if let Some(u) = used.get_mut(j) {
                        *u = true;
                    }
                    pairs.push((i, j));
                }
            }
        }
        let cx = self.center[0];
        for (a, b) in pairs {
            let (Some(pa), Some(pb)) = (self.pts.get(a).copied(), self.pts.get(b).copied()) else { continue };
            let avg_a = [(pa[0] + 2.0 * cx - pb[0]) * 0.5, (pa[1] + pb[1]) * 0.5];
            let avg_b = [2.0 * cx - avg_a[0], avg_a[1]];
            let new_a = if t > 0.0 {
                [pa[0] + (avg_a[0] - pa[0]) * t, pa[1] + (avg_a[1] - pa[1]) * t]
            } else {
                let k = -t;
                [pa[0] + (pa[0] - avg_a[0]) * k, pa[1] + (pa[1] - avg_a[1]) * k]
            };
            let new_b = if t > 0.0 {
                [pb[0] + (avg_b[0] - pb[0]) * t, pb[1] + (avg_b[1] - pb[1]) * t]
            } else {
                let k = -t;
                [pb[0] + (pb[0] - avg_b[0]) * k, pb[1] + (pb[1] - avg_b[1]) * k]
            };
            if let Some(p) = self.pts.get_mut(a) {
                *p = new_a;
            }
            if let Some(p) = self.pts.get_mut(b) {
                *p = new_b;
            }
        }
    }
}

/// 478 face points plus the 6 derived neck controls.
static ALL_FACE_PLUS_DERIVED: OnceLock<Vec<usize>> = OnceLock::new();

fn all_face_plus_derived() -> &'static [usize] {
    ALL_FACE_PLUS_DERIVED.get_or_init(|| (0..484).collect())
}

fn centroid_of(points: &[[f64; 2]], group: &[usize]) -> [f64; 2] {
    let mut sum = [0.0, 0.0];
    let mut n = 0.0;
    let mut seen = vec![false; points.len()];
    for &i in group {
        if seen.get(i) == Some(&true) {
            continue;
        }
        if let Some(slot) = seen.get_mut(i) {
            *slot = true;
        }
        if let Some(p) = points.get(i) {
            sum[0] += p[0];
            sum[1] += p[1];
            n += 1.0;
        }
    }
    if n > 0.0 { [sum[0] / n, sum[1] / n] } else { [0.0, 0.0] }
}

/// Greedy nearest mirrored partner for each point; used by every symmetry slider.
fn mirror_pairs(points: &[[f64; 2]], cx: f64, tolerance: f64) -> Vec<(usize, usize)> {
    let mut used = vec![false; points.len()];
    let mut pairs = Vec::new();
    for i in 0..points.len() {
        if used.get(i) == Some(&true) {
            continue;
        }
        let Some(pi) = points.get(i) else { continue };
        let target = [2.0 * cx - pi[0], pi[1]];
        let mut best: Option<(usize, f64)> = None;
        for j in (i + 1)..points.len() {
            if used.get(j) == Some(&true) {
                continue;
            }
            let Some(pj) = points.get(j) else { continue };
            let dist = (pj[0] - target[0]).hypot(pj[1] - target[1]);
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((j, dist));
            }
        }
        if let Some((j, dist)) = best
            && dist <= tolerance
        {
            if let Some(u) = used.get_mut(i) {
                *u = true;
            }
            if let Some(u) = used.get_mut(j) {
                *u = true;
            }
            pairs.push((i, j));
        }
    }
    pairs
}

// ---------------------------------------------------------------------------------------------
// Displacement field
// ---------------------------------------------------------------------------------------------

/// A coarse displacement grid over the face region, bilinearly sampled by the warp map.
struct DisplacementGrid {
    rect: Rect,
    w: usize,
    h: usize,
    data: Vec<[f32; 2]>,
}

impl DisplacementGrid {
    fn build(region: Rect, geometry: &Geometry) -> Self {
        let w = ((region.width() / GRID_PIXELS_PER_CELL).clamp(GRID_MIN as u32, GRID_MAX as u32)) as usize;
        let h = ((region.height() / GRID_PIXELS_PER_CELL).clamp(GRID_MIN as u32, GRID_MAX as u32)) as usize;
        let mut data = vec![[0.0f32; 2]; w.saturating_mul(h)];
        if region.is_empty() || w == 0 || h == 0 {
            return Self { rect: region, w, h, data };
        }
        let radius = (geometry.size * f64::from(KERNEL)).max(4.0);
        let r2 = radius * radius;
        // Control points carry their own displacement. Unmoved landmarks are zero anchors, so a
        // slider that moves one group retains the rest of the face.
        let mut controls: Vec<([f64; 2], [f64; 2])> = Vec::with_capacity(geometry.pts.len() + 64);
        for (i, base) in geometry.base.iter().enumerate() {
            let moved = geometry.pts.get(i).copied().unwrap_or(*base);
            let delta = [moved[0] - base[0], moved[1] - base[1]];
            if delta[0] != 0.0 || delta[1] != 0.0 || i % 7 == 0 {
                controls.push((*base, delta));
            }
        }
        // Perimeter anchors keep the field at zero at the edge of the region.
        let step = (radius * 1.5).max(6.0);
        let (x0, y0, x1, y1) = (f64::from(region.x0), f64::from(region.y0), f64::from(region.x1), f64::from(region.y1));
        let mut x = x0;
        while x <= x1 {
            controls.push(([x, y0], [0.0, 0.0]));
            controls.push(([x, y1], [0.0, 0.0]));
            x += step;
        }
        let mut y = y0;
        while y <= y1 {
            controls.push(([x0, y], [0.0, 0.0]));
            controls.push(([x1, y], [0.0, 0.0]));
            y += step;
        }
        let fade = (f64::from(region.width().min(region.height())) * f64::from(EDGE_FADE)).max(4.0);
        for gy in 0..h {
            let py = f64::from(region.y0) + (gy as f64 + 0.5) * f64::from(region.height()) / h as f64;
            for gx in 0..w {
                let px = f64::from(region.x0) + (gx as f64 + 0.5) * f64::from(region.width()) / w as f64;
                let mut acc = [0.0, 0.0];
                let mut total = 0.0;
                for (at, delta) in &controls {
                    let dx = px - at[0];
                    let dy = py - at[1];
                    let d2 = dx * dx + dy * dy;
                    if d2 >= r2 {
                        continue;
                    }
                    let k = 1.0 - d2 / r2;
                    let weight = k * k * k;
                    acc[0] += delta[0] * weight;
                    acc[1] += delta[1] * weight;
                    total += weight;
                }
                if total > 1e-12 {
                    let edge = (px - x0).min(x1 - px).min(py - y0).min(y1 - py).max(0.0);
                    let fade_k = smoothstep((edge / fade).min(1.0));
                    let o = gy * w + gx;
                    if let Some(cell) = data.get_mut(o) {
                        // A tiny delta from rounding in an identity warp must stay exactly zero.
                        let dx = acc[0] / total * fade_k;
                        let dy = acc[1] / total * fade_k;
                        if dx.abs() > 1e-9 || dy.abs() > 1e-9 {
                            cell[0] = dx as f32;
                            cell[1] = dy as f32;
                        }
                    }
                }
            }
        }
        Self { rect: region, w, h, data }
    }

    fn at(&self, x: f64, y: f64) -> (f64, f64) {
        if self.w == 0 || self.h == 0 || self.rect.is_empty() || !self.rect.contains(x.floor() as i32, y.floor() as i32) {
            return (0.0, 0.0);
        }
        let fx = ((x - f64::from(self.rect.x0)) * self.w as f64 / f64::from(self.rect.width())) - 0.5;
        let fy = ((y - f64::from(self.rect.y0)) * self.h as f64 / f64::from(self.rect.height())) - 0.5;
        let (ix, iy) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - ix).clamp(0.0, 1.0), (fy - iy).clamp(0.0, 1.0));
        let clamp = |v: f64, n: usize| v.clamp(0.0, n.saturating_sub(1) as f64) as usize;
        let (x0, y0) = (clamp(ix, self.w), clamp(iy, self.h));
        let (x1, y1) = (clamp(ix + 1.0, self.w), clamp(iy + 1.0, self.h));
        let sample = |xx: usize, yy: usize| self.data.get(yy * self.w + xx).copied().unwrap_or([0.0, 0.0]);
        let (a, b, c, d) = (sample(x0, y0), sample(x1, y0), sample(x0, y1), sample(x1, y1));
        let top = [f64::from(a[0]) * (1.0 - tx) + f64::from(b[0]) * tx, f64::from(a[1]) * (1.0 - tx) + f64::from(b[1]) * tx];
        let bottom = [f64::from(c[0]) * (1.0 - tx) + f64::from(d[0]) * tx, f64::from(c[1]) * (1.0 - tx) + f64::from(d[1]) * tx];
        (top[0] * (1.0 - ty) + bottom[0] * ty, top[1] * (1.0 - ty) + bottom[1] * ty)
    }
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------------------------
// Detection and landmarks
// ---------------------------------------------------------------------------------------------

/// One face and its 478 landmarks, in document pixels.
#[derive(Clone)]
struct FaceInput {
    rect: Rect,
    points: Vec<[f32; 3]>,
}

/// Which detected faces `retouchAI.faceReshape` warps.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FaceSelection {
    /// The best face (the detector's first).
    Main,
    /// Every detected face.
    All,
    /// Several 1-based faces in the detector's (best-first) order, unique and sorted.
    Selected(Vec<usize>),
}

impl FaceSelection {
    fn parse(p: &Value) -> Result<Self> {
        let bad_faces = || bad("`faces` must be \"main\", \"all\", a 1-based face number or a non-empty array of face numbers");
        let one = |v: &Value| -> Option<usize> {
            match v {
                Value::Number(n) => n.as_u64().filter(|n| *n >= 1).map(|n| n as usize),
                Value::String(s) => s.parse::<usize>().ok().filter(|n| *n >= 1),
                _ => None,
            }
        };
        match p.get("faces") {
            None | Some(Value::Null) => Ok(Self::Main),
            Some(Value::String(s)) if s == "main" => Ok(Self::Main),
            Some(Value::String(s)) if s == "all" => Ok(Self::All),
            Some(v @ (Value::Number(_) | Value::String(_))) => one(v).map(|n| Self::Selected(vec![n])).ok_or_else(bad_faces),
            Some(Value::Array(items)) => {
                if items.is_empty() {
                    return Err(bad("`faces` must name at least one face"));
                }
                let mut selected = Vec::with_capacity(items.len());
                for item in items {
                    selected.push(one(item).ok_or_else(bad_faces)?);
                }
                selected.sort_unstable();
                selected.dedup();
                Ok(Self::Selected(selected))
            }
            Some(_) => Err(bad_faces()),
        }
    }

    /// 0-based indices into `count` detected faces; out-of-range numbers are clear errors.
    fn indices(&self, count: usize) -> Result<Vec<usize>> {
        match self {
            Self::Main => {
                if count == 0 {
                    return Err(bad("no face was detected"));
                }
                Ok(vec![0])
            }
            Self::All => Ok((0..count).collect()),
            Self::Selected(faces) => {
                for n in faces {
                    if *n > count {
                        return Err(bad(format!("face {n} was not found; the detector found {count}")));
                    }
                }
                Ok(faces.iter().map(|n| n - 1).collect())
            }
        }
    }

    /// 1-based indices in detector order, for the result payload.
    fn selected_numbers(&self, count: usize) -> Vec<usize> {
        self.indices(count).map(|indices| indices.into_iter().map(|i| i + 1).collect()).unwrap_or_default()
    }

    fn id(&self) -> String {
        match self {
            Self::Main => "main".into(),
            Self::All => "all".into(),
            Self::Selected(faces) => faces.iter().map(usize::to_string).collect::<Vec<_>>().join(","),
        }
    }
}

/// The selected faces with their landmarks, plus how many faces the detector found in total.
struct FaceInputs {
    selected: Vec<FaceInput>,
    detected: usize,
}

#[cfg(test)]
thread_local! {
    static TEST_FACES: std::cell::RefCell<Option<Vec<FaceInput>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn inject_faces(faces: Vec<FaceInput>) {
    TEST_FACES.with(|cell| *cell.borrow_mut() = Some(faces));
}

#[cfg(test)]
fn inject_face(face: FaceInput) {
    inject_faces(vec![face]);
}

#[cfg(test)]
fn clear_injected_face() {
    TEST_FACES.with(|cell| *cell.borrow_mut() = None);
}

#[cfg(test)]
fn injected_faces() -> Option<Vec<FaceInput>> {
    TEST_FACES.with(|cell| cell.borrow().clone())
}

fn analysis_size(canvas: Rect) -> (usize, usize) {
    let (w, h) = (canvas.width().max(1), canvas.height().max(1));
    let scale = (ANALYSIS_SIDE as f32 / w.max(h) as f32).min(1.0);
    let aw = ((w as f32 * scale).round() as usize).max(1).min(w as usize);
    let ah = ((h as f32 * scale).round() as usize).max(1).min(h as usize);
    (aw, ah)
}

/// A clone of `doc` with only `source` (and its ancestors) visible: the detector sees the layer
/// the user is reshaping, not whatever happens to be above it.
fn source_only_document(doc: &Document, source: LayerId) -> Document {
    let mut reduced = doc.clone();
    let paths: Vec<_> = reduced.walk().into_iter().map(|(p, _, _)| p).collect();
    for path in paths {
        if let Some(layer) = reduced.layer_at_mut(&path) {
            layer.visible = false;
        }
    }
    if let Some(path) = reduced.path_of(source) {
        for end in 1..=path.len() {
            if let Some(layer) = reduced.layer_at_mut(&path[..end]) {
                layer.visible = true;
            }
        }
    }
    reduced
}

fn rect_from_face(rect: [f32; 4], canvas: Rect) -> Rect {
    if !rect.iter().all(|v| v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 {
        return Rect::EMPTY;
    }
    let r = Rect::from_xywh(
        rect[0].floor() as i32,
        rect[1].floor() as i32,
        rect[2].max(0.0).ceil().min(f32::from(u16::MAX)) as u32,
        rect[3].max(0.0).ceil().min(f32::from(u16::MAX)) as u32,
    );
    r.intersect(&canvas)
}

fn face_region(face: Rect, canvas: Rect) -> Rect {
    if face.is_empty() {
        return Rect::EMPTY;
    }
    let w = face.width() as f32;
    let h = face.height() as f32;
    let x0 = (face.x0 as f32 - w * REGION_LEFT).floor() as i32;
    let y0 = (face.y0 as f32 - h * REGION_TOP).floor() as i32;
    let x1 = (face.x1 as f32 + w * REGION_RIGHT).ceil() as i32;
    let y1 = (face.y1 as f32 + h * REGION_BOTTOM).ceil() as i32;
    Rect::new(x0, y0, x1, y1).intersect(&canvas)
}

/// Detect every face on a reduced, source-only render. The boxes stay in document pixels; the
/// (more expensive) landmark model runs only for the faces the caller selects.
fn detect_face_boxes(doc: &Document, source: LayerId) -> Result<Vec<Rect>> {
    if !faces::AVAILABLE {
        return Err(EngineError::Other("face detection isn't available in the browser version of PhotoCraft; use the desktop app".into()));
    }
    if !face_mesh::AVAILABLE {
        return Err(EngineError::Other("face landmarks aren't available in the browser version of PhotoCraft; use the desktop app".into()));
    }
    let canvas = doc.bounds();
    if canvas.is_empty() {
        return Err(bad("the document has no pixels"));
    }
    let layer = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
    if layer.surface().is_none() {
        return Err(bad(format!("layer \"{}\" has no pixels", layer.name)));
    }
    let layer_name = layer.name.clone();
    let (aw, ah) = analysis_size(canvas);
    let proxy = source_only_document(doc, source);
    let buffer = photocraft_compose::render_reduced(&proxy, aw as u32, ah as u32);
    if buffer.px.len() != aw.saturating_mul(ah) {
        return Err(EngineError::Other("the reduced face-detection image has an unexpected size".into()));
    }
    let found = faces::detect(&buffer.px, aw, ah, MIN_SCORE).ok_or_else(|| EngineError::Other("face detection failed".into()))?;
    if found.is_empty() {
        return Err(bad(format!("no face was found in layer \"{layer_name}\"; Face Reshape needs a visible face")));
    }
    let sx = canvas.width() as f32 / aw as f32;
    let sy = canvas.height() as f32 / ah as f32;
    let boxes: Vec<Rect> = found
        .iter()
        .map(|face| rect_from_face([canvas.x0 as f32 + face.rect[0] * sx, canvas.y0 as f32 + face.rect[1] * sy, face.rect[2] * sx, face.rect[3] * sy], canvas))
        .filter(|rect| !rect.is_empty())
        .collect();
    if boxes.is_empty() {
        return Err(bad("the detected faces do not overlap the document"));
    }
    Ok(boxes)
}

/// Run the 478-point landmark model on a full-resolution crop around one detected box. Coordinates
/// are document pixels.
fn landmarks_for(doc: &Document, source: LayerId, rect: Rect) -> Result<FaceInput> {
    let canvas = doc.bounds();
    let layer = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
    let surface = layer.surface().ok_or_else(|| bad(format!("layer \"{}\" has no pixels", layer.name)))?;
    // CROP_PADDING in the landmark model is 1.5x the box's longest side; 1.6x here leaves a
    // margin so the model's own crop never touches the buffer edge.
    let side = rect.width().max(rect.height()) as f32;
    let pad = side * 0.30;
    let crop = Rect::new(
        (rect.x0 as f32 - pad).floor() as i32,
        (rect.y0 as f32 - pad).floor() as i32,
        (rect.x1 as f32 + pad).ceil() as i32,
        (rect.y1 as f32 + pad).ceil() as i32,
    )
    .intersect(&canvas);
    if crop.is_empty() {
        return Err(bad("the detected face lies outside the document"));
    }
    let (cw, ch) = (crop.width() as usize, crop.height() as usize);
    let pixels = cw.checked_mul(ch).ok_or_else(|| bad("the face crop is too large"))?;
    let mut rgba = vec![[0.0f32; 4]; pixels];
    surface.read_rgba_into(crop, &mut rgba);
    let local = [rect.x0 as f32 - crop.x0 as f32, rect.y0 as f32 - crop.y0 as f32, rect.width() as f32, rect.height() as f32];
    let mut points = face_mesh::landmarks(&rgba, cw, ch, local).map_err(|e| EngineError::Other(format!("face landmarks failed: {e}")))?;
    for point in &mut points {
        point[0] += crop.x0 as f32;
        point[1] += crop.y0 as f32;
    }
    Ok(FaceInput { rect, points })
}

fn face_inputs(s: &Session, source: LayerId, selection: &FaceSelection) -> Result<FaceInputs> {
    #[cfg(test)]
    if let Some(faces) = injected_faces() {
        let detected = faces.len();
        let indices = selection.indices(detected)?;
        let selected = indices.into_iter().filter_map(|i| faces.get(i).cloned()).collect();
        return Ok(FaceInputs { selected, detected });
    }
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let boxes = detect_face_boxes(&doc, source)?;
    let detected = boxes.len();
    let indices = selection.indices(detected)?;
    let selected = indices
        .into_iter()
        .map(|i| boxes.get(i).copied().ok_or_else(|| bad(format!("face {} was not found", i + 1))).and_then(|rect| landmarks_for(&doc, source, rect)))
        .collect::<Result<Vec<_>>>()?;
    Ok(FaceInputs { selected, detected })
}

// ---------------------------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------------------------

fn parse_bool(p: &Value, key: &str, default: bool) -> Result<bool> {
    match p.get(key) {
        None => Ok(default),
        Some(Value::Bool(v)) => Ok(*v),
        Some(_) => Err(bad(format!("`{key}` must be true or false"))),
    }
}

fn parse_preview_layer(p: &Value) -> Result<Option<LayerId>> {
    match p.get("previewLayer") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n.as_u64().map(|id| Some(LayerId(id))).ok_or_else(|| bad("`previewLayer` must be a layer id")),
        Some(_) => Err(bad("`previewLayer` must be a layer id")),
    }
}

/// The pixel layer to reshape. Like `layer_param`, but if the active layer is one of this tool's
/// own result layers (a user selected the preview), use the nearest pixel layer below it: the
/// preview was made from that layer and the next slider drag must keep using it.
fn source_layer(s: &Session, p: &Value) -> Result<LayerId> {
    if let Some(id) = crate::commands::explicit_layer(s, p)? {
        return Ok(id);
    }
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let active = d.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
    let Some(layer) = d.doc.layer(active) else {
        return Ok(active);
    };
    if layer.name != PREVIEW_NAME && layer.name != FINAL_NAME {
        return Ok(active);
    }
    let Some(path) = d.doc.path_of(active) else {
        return Ok(active);
    };
    let Some((&last, parent)) = path.split_last() else {
        return Ok(active);
    };
    for index in (0..last).rev() {
        let mut candidate = parent.to_vec();
        candidate.push(index);
        if let Some(layer) = d.doc.layer_at(&candidate)
            && matches!(layer.content, LayerContent::Raster(_))
        {
            return Ok(layer.id);
        }
    }
    Ok(active)
}

fn named_layer(doc: &Document, name: &str, exclude: LayerId) -> Option<LayerId> {
    doc.walk().into_iter().rev().find(|(_, _, l)| l.id != exclude && l.name == name && matches!(l.content, LayerContent::Raster(_))).map(|(_, _, l)| l.id)
}

fn preview_layer(doc: &Document, source: LayerId, explicit: Option<LayerId>) -> Result<Option<LayerId>> {
    if let Some(id) = explicit {
        let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        if layer.name != PREVIEW_NAME || !matches!(layer.content, LayerContent::Raster(_)) {
            return Err(bad(format!("layer {} is not a \"{PREVIEW_NAME}\" pixel layer", id.0)));
        }
        return Ok(Some(id));
    }
    Ok(named_layer(doc, PREVIEW_NAME, source))
}

fn final_layer(doc: &Document, source: LayerId) -> Option<LayerId> {
    named_layer(doc, FINAL_NAME, source)
}

fn write_layer(doc: &mut Document, source: LayerId, id: Option<LayerId>, name: &str, pixels: Surface, visible: bool) -> Result<LayerId> {
    let id = match id {
        Some(id) => {
            let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
            layer.content = LayerContent::Raster(pixels);
            layer.name = name.to_string();
            layer.visible = visible;
            id
        }
        None => {
            let format = pixels.format();
            let mut layer = Layer::raster(name, format);
            layer.content = LayerContent::Raster(pixels);
            layer.visible = visible;
            doc.insert_above(Some(source), layer)
        }
    };
    Ok(id)
}

fn result(layer: Option<LayerId>, name: &str, preview: bool, compare: bool, face: [f32; 4], changed: bool) -> Value {
    json!({
        "layer": layer.map(|id| id.0),
        "name": name,
        "preview": preview,
        "compare": compare,
        "face": face,
        "faces": [],
        "detected": 0,
        "selected": [],
        "points": face_mesh::POINTS,
        "changed": changed,
    })
}

fn face_box_json(face: Rect) -> [f32; 4] {
    [face.x0 as f32, face.y0 as f32, face.width() as f32, face.height() as f32]
}

/// Reset and compare never need detection or the landmark model: they only touch the preview
/// layer's presence and visibility.
fn preview_only(s: &mut Session, p: &Value, reset: bool, compare: bool, has_params: bool) -> Result<Option<Value>> {
    let source = source_layer(s, p)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let preview_id = preview_layer(&doc, source, parse_preview_layer(p)?)?;
    if reset {
        let changed = if let Some(id) = preview_id {
            s.edit("Reset Face Reshape", |doc, active| {
                doc.remove(id);
                if *active == Some(id) {
                    *active = Some(source);
                }
                Ok(())
            })?;
            true
        } else {
            false
        };
        return Ok(Some(result(None, "", false, compare, [0.0, 0.0, 0.0, 0.0], changed)));
    }
    // A compare-only call (no `params` object at all) must not remove an all-zero preview.
    if !has_params && p.get("compare").is_some() {
        let Some(id) = preview_id else {
            return Ok(Some(result(None, "", false, compare, [0.0, 0.0, 0.0, 0.0], false)));
        };
        let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let want = !compare;
        let changed = layer.visible != want;
        if changed {
            s.edit("Compare Face Reshape", |doc, _| {
                if let Some(layer) = doc.layer_mut(id) {
                    layer.visible = want;
                }
                Ok(())
            })?;
        }
        return Ok(Some(result(Some(id), PREVIEW_NAME, true, compare, [0.0, 0.0, 0.0, 0.0], changed)));
    }
    if has_params {
        return Ok(None);
    }
    // No params and no compare: only a reset is meaningful (handled above). Leaving an existing
    // preview alone is the least surprising behaviour for a bare call.
    if let Some(id) = preview_id {
        return Ok(Some(result(Some(id), PREVIEW_NAME, true, compare, [0.0, 0.0, 0.0, 0.0], false)));
    }
    Ok(Some(result(None, "", false, compare, [0.0, 0.0, 0.0, 0.0], false)))
}

// ---------------------------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------------------------

fn face_reshape_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let reset = parse_bool(p, "reset", false)?;
    let compare = parse_bool(p, "compare", false)?;
    let preview = parse_bool(p, "preview", true)?;
    let has_params = p.get("params").is_some_and(|v| !v.is_null());
    let params = match p.get("params") {
        None | Some(Value::Null) => FaceReshapeParams::default(),
        Some(value) => FaceReshapeParams::from_value(value).map_err(bad)?,
    };
    if let Some(answer) = preview_only(s, p, reset, compare, has_params)? {
        return Ok(answer);
    }
    let selection = FaceSelection::parse(p)?;
    let source = source_layer(s, p)?;
    let doc = s.active().ok_or(EngineError::NoDocument)?.doc.clone();
    let layer = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
    if !matches!(layer.content, LayerContent::Raster(_)) {
        return Err(bad(format!("layer \"{}\" is not a pixel layer", layer.name)));
    }
    let explicit_preview = parse_preview_layer(p)?;
    let old_preview = preview_layer(&doc, source, explicit_preview)?;
    let old_final = final_layer(&doc, source);

    if params.is_identity() {
        // All sliders at zero: a leftover preview would show an old deformation, so clear it.
        let changed = if let Some(id) = old_preview {
            s.edit("Clear Face Reshape", |doc, active| {
                doc.remove(id);
                if *active == Some(id) {
                    *active = Some(source);
                }
                Ok(())
            })?;
            true
        } else {
            false
        };
        return Ok(result(None, "", false, compare, [0.0, 0.0, 0.0, 0.0], changed));
    }

    let input = face_inputs(s, source, &selection)?;
    if input.selected.is_empty() {
        return Err(bad("no face was selected"));
    }
    let detected = input.detected;
    let selected_numbers = selection.selected_numbers(detected);
    let face = input.selected[0].rect;
    let canvas = doc.bounds();
    let source_surface = layer.surface().ok_or_else(|| bad(format!("layer \"{}\" has no pixels", layer.name)))?;
    let mut grids = Vec::with_capacity(input.selected.len());
    let mut face_boxes = Vec::with_capacity(input.selected.len());
    let mut region = Rect::EMPTY;
    for input in &input.selected {
        let face = input.rect;
        let mut geometry = Geometry::from_landmarks(&input.points, face).map_err(bad)?;
        geometry.apply(&params);
        let face_region = face_region(face, canvas);
        if face_region.is_empty() {
            return Err(bad("the face is outside the document"));
        }
        grids.push(DisplacementGrid::build(face_region, &geometry));
        region = region.union(&face_region);
        face_boxes.push(face);
    }
    // One warp over the union of the faces' regions: a single layer, no tile seams where two
    // padded regions share a tile. Displacements from every face grid add where regions overlap.
    let map = |x: f64, y: f64| {
        let (mut dx, mut dy) = (0.0, 0.0);
        for grid in &grids {
            let (gx, gy) = grid.at(x, y);
            dx += gx;
            dy += gy;
        }
        (x + dx, y + dy)
    };
    let warped = warp_mesh_surface(source_surface, region, &map, Interp::Bicubic);
    if warped.content_bounds().is_empty() {
        return Err(bad("the face region has no pixels to reshape"));
    }

    let (target_name, label) = if preview { (PREVIEW_NAME, "Face Reshape Preview") } else { (FINAL_NAME, "Apply Face Reshape") };
    let target_id = if preview { old_preview } else { old_final.or(old_preview) };
    let visible = !compare;
    let id = s.edit(label, |doc, _active| {
        let id = write_layer(doc, source, target_id, target_name, warped, visible)?;
        if !preview {
            // Apply collapses the preview into the final layer, so no duplicate remains.
            if let Some(preview_id) = old_preview
                && preview_id != id
            {
                doc.remove(preview_id);
            }
        }
        // Keep the source layer active: the panel sends slider drags without a `layer` param and
        // the next call must find the same source, not the preview it just made.
        Ok(id)
    })?;
    let face = face_box_json(face);
    let mut out = result(Some(id), target_name, preview, compare, face, true);
    out["faces"] = json!(face_boxes.iter().map(|face| face_box_json(*face)).collect::<Vec<_>>());
    out["detected"] = json!(detected);
    out["selected"] = json!(selected_numbers);
    out["faceMode"] = json!(selection.id());
    Ok(out)
}

fn has_face_source(s: &Session) -> std::result::Result<(), String> {
    if !face_mesh::AVAILABLE {
        return Err("face landmarks aren't available in the browser version of PhotoCraft; use the desktop app".into());
    }
    let d = s.active().ok_or("no document open")?;
    let l = d.active_layer.and_then(|id| d.doc.layer(id)).ok_or("no active layer")?;
    if matches!(l.content, LayerContent::Raster(_)) {
        Ok(())
    } else {
        Err(format!("active layer is {} {} layer, not a pixel layer", l.content.article(), l.content.kind_name()))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Face Reshape",
        menu: &[],
        shortcut: None,
        params: r##"{"layer":id? (pixel source; default active layer),"params":{"head_scale": -100..100=0,"cranium_height": -100..100=0,"hair_volume": -100..100=0,"hairline_shift": -100..100=0,"face_slim": -100..100=0,"face_width": -100..100=0,"face_length": -100..100=0,"jaw_width": -100..100=0,"jaw_angle": -100..100=0,"cheekbone_shrink": -100..100=0,"cheek_slim": -100..100=0,"temple_fill": -100..100=0,"chin_length": -100..100=0,"chin_width": -100..100=0,"chin_protrusion": -100..100=0,"chin_cleft": -100..100=0,"double_chin_warp": -100..100=0,"eye_size": -100..100=0,"eye_width": -100..100=0,"eye_height": -100..100=0,"eye_distance": -100..100=0,"eye_tilt": -100..100=0,"eye_position_y": -100..100=0,"inner_canthus": -100..100=0,"outer_canthus": -100..100=0,"lower_eyelid_down": -100..100=0,"pupil_scale": -100..100=0,"aegyo_sal": -100..100=0,"eye_brow_distance": -100..100=0,"eyelid_fold_width": -100..100=0,"eyebrow_height": -100..100=0,"eyebrow_distance": -100..100=0,"eyebrow_length": -100..100=0,"eyebrow_thickness": -100..100=0,"eyebrow_arch": -100..100=0,"eyebrow_tail": -100..100=0,"eyebrow_tilt": -100..100=0,"nose_scale": -100..100=0,"nose_wing_width": -100..100=0,"nose_tip_size": -100..100=0,"nose_tip_angle": -100..100=0,"nose_length": -100..100=0,"bridge_width": -100..100=0,"bridge_curvature": -100..100=0,"nasal_hump_reduction": -100..100=0,"columella_height": -100..100=0,"radix_height": -100..100=0,"nostril_size": -100..100=0,"mouth_scale": -100..100=0,"mouth_width": -100..100=0,"mouth_position_y": -100..100=0,"philtrum_length": -100..100=0,"upper_lip_thickness": -100..100=0,"lower_lip_thickness": -100..100=0,"smile_corners": -100..100=0,"m_lip_shape": -100..100=0,"lip_plumpness": -100..100=0,"lip_protrusion": -100..100=0,"lower_lip_to_chin": -100..100=0,"teeth_align": -100..100=0,"three_section_ratio": -100..100=0,"face_symmetry": -100..100=0,"eye_symmetry": -100..100=0,"eyebrow_symmetry": -100..100=0,"neck_slim": -100..100=0,"neck_length": -100..100=0,"e_line_adjustment": -100..100=0,"midface_projection": -100..100=0},"faces":"main"|"all"|n|[n,…]="main","preview":bool=true,"previewLayer":id?,"compare":bool=false,"reset":bool=false} → {"layer":id?,"name":str,"preview":bool,"compare":bool,"face":[x,y,w,h],"faces":[[x,y,w,h],…],"detected":n,"selected":[n,…],"faceMode":str,"points":478,"changed":bool} (non-destructive: source pixels and history stay intact; preview:true makes a "Face Reshape Preview" layer above the source, preview:false renames/creates "Face Reshape", compare hides/shows the preview, reset removes it; pass the same "coalesce" key from a slider drag for one history step; `faces` arrays are 1-based in the detector's best-first order and all selected faces land in one layer)"##,
        enabled: has_face_source,
        run: face_reshape_cmd,
        journal: true,
    }]
}

// ---------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{Color, ColorMode, SampleType};
    use photocraft_doc::Size;
    use serde_json::json;

    fn synthetic_rgba(w: usize, h: usize) -> Vec<[f32; 4]> {
        let mut px = vec![[0.96f32, 0.96, 0.96, 1.0]; w * h];
        let cx = w as f32 * 0.5;
        let cy = h as f32 * 0.48;
        let rx = w as f32 * 0.23;
        let ry = h as f32 * 0.30;
        for y in 0..h {
            for x in 0..w {
                let dx = (x as f32 - cx) / rx;
                let dy = (y as f32 - cy) / ry;
                if dx * dx + dy * dy <= 1.0 {
                    px[y * w + x] = [0.82, 0.60, 0.47, 1.0];
                }
            }
        }
        // Eyes, brows, nose and mouth: enough edges for the warp to be measurable.
        for (x, y, rw, rh, c) in [
            (cx - w as f32 * 0.09, cy - h as f32 * 0.06, w as f32 * 0.055, h as f32 * 0.020, [0.12, 0.09, 0.08, 1.0]),
            (cx + w as f32 * 0.09, cy - h as f32 * 0.06, w as f32 * 0.055, h as f32 * 0.020, [0.12, 0.09, 0.08, 1.0]),
            (cx, cy + h as f32 * 0.08, w as f32 * 0.035, h as f32 * 0.030, [0.60, 0.38, 0.30, 1.0]),
            (cx, cy + h as f32 * 0.19, w as f32 * 0.075, h as f32 * 0.014, [0.45, 0.18, 0.18, 1.0]),
        ] {
            for yy in 0..h {
                for xx in 0..w {
                    let dx = (xx as f32 - x) / rw;
                    let dy = (yy as f32 - y) / rh;
                    if dx * dx + dy * dy <= 1.0 {
                        px[yy * w + xx] = c;
                    }
                }
            }
        }
        // A fine luminance texture across the whole frame: even a deformation inside a flat
        // skin or eye area must move *some* output pixel for the per-slider test below.
        for y in 0..h {
            for x in 0..w {
                let t = ((x * 17 + y * 31) % 23) as f32 / 23.0 - 0.5;
                let p = &mut px[y * w + x];
                for c in p.iter_mut().take(3) {
                    *c = (*c + t * 0.06).clamp(0.0, 1.0);
                }
            }
        }
        px
    }

    fn session_with_face() -> Session {
        session_with_face_at(SampleType::U8)
    }

    fn session_with_face_at(depth: SampleType) -> Session {
        let size = Size::new(192, 192);
        let mut doc = Document::with_background("Face Reshape test", size, ColorMode::Rgb, depth, Color::WHITE);
        let rgba = synthetic_rgba(192, 192);
        let fmt = doc.pixel_format();
        let bounds = doc.bounds();
        let ch = fmt.channels();
        let mut native = vec![0.0f32; rgba.len() * ch];
        let mut tmp = [0.0f32; 4];
        for (i, p) in rgba.iter().enumerate() {
            let _ = photocraft_raster::from_rgba_into(&fmt, *p, &mut tmp);
            native[i * ch..(i + 1) * ch].copy_from_slice(&tmp[..ch]);
        }
        if let Some(surface) = doc.layers[0].surface_mut() {
            surface.write_region(bounds, &native);
        }
        let mut s = Session::new();
        s.add_document(doc, None);
        s
    }

    fn blank_session() -> Session {
        let doc = Document::with_background("Blank", Size::new(96, 80), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let mut s = Session::new();
        s.add_document(doc, None);
        s
    }

    fn source_id(s: &Session) -> LayerId {
        s.active().and_then(|d| d.active_layer).unwrap()
    }

    fn pixels(s: &Session, id: LayerId) -> Vec<[f32; 4]> {
        let d = s.active().unwrap();
        let r = d.doc.bounds();
        let mut out = vec![[0.0f32; 4]; r.width() as usize * r.height() as usize];
        if let Some(surface) = d.doc.layer(id).and_then(|l| l.surface()) {
            surface.read_rgba_into(r, &mut out);
        }
        out
    }

    fn names(s: &Session) -> Vec<String> {
        s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l.name.clone()).collect()
    }

    fn test_face() -> FaceInput {
        static FACE: OnceLock<FaceInput> = OnceLock::new();
        FACE.get_or_init(|| {
            let w = 192;
            let h = 192;
            let px = synthetic_rgba(w, h);
            let rect = Rect::from_xywh(38, 34, 116, 116);
            let points = face_mesh::landmarks(&px, w, h, [rect.x0 as f32, rect.y0 as f32, rect.width() as f32, rect.height() as f32])
                .expect("the bundled landmark model must run");
            FaceInput { rect, points }
        })
        .clone()
    }

    /// Two faces in detector order: the first at x 38, the second shifted 200 px right (far
    /// enough apart that their padded warp regions do not overlap).
    fn test_faces() -> Vec<FaceInput> {
        let first = test_face();
        let mut second = first.clone();
        second.rect = first.rect.translate(200, 0);
        for point in &mut second.points {
            point[0] += 200.0;
        }
        vec![first, second]
    }

    fn session_with_two_faces() -> Session {
        let size = Size::new(384, 192);
        let mut doc = Document::with_background("Two Face Reshape test", size, ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let rgba = synthetic_rgba(384, 192);
        let fmt = doc.pixel_format();
        let bounds = doc.bounds();
        let ch = fmt.channels();
        let mut native = vec![0.0f32; rgba.len() * ch];
        let mut tmp = [0.0f32; 4];
        for (i, p) in rgba.iter().enumerate() {
            let _ = photocraft_raster::from_rgba_into(&fmt, *p, &mut tmp);
            native[i * ch..(i + 1) * ch].copy_from_slice(&tmp[..ch]);
        }
        if let Some(surface) = doc.layers[0].surface_mut() {
            surface.write_region(bounds, &native);
        }
        let mut s = Session::new();
        s.add_document(doc, None);
        s
    }

    fn execute_with_face(s: &mut Session, p: Value) -> Result<Value> {
        inject_face(test_face());
        let r = s.execute(CMD, p);
        clear_injected_face();
        r
    }

    fn execute_with_faces(s: &mut Session, p: Value) -> Result<Value> {
        inject_faces(test_faces());
        let r = s.execute(CMD, p);
        clear_injected_face();
        r
    }

    fn region_changed(a: &[[f32; 4]], b: &[[f32; 4]], w: usize, region: Rect) -> bool {
        for y in region.y0.max(0)..region.y1.max(0) {
            for x in region.x0.max(0)..region.x1.max(0) {
                let i = y as usize * w + x as usize;
                if let (Some(p), Some(q)) = (a.get(i), b.get(i))
                    && q[3] > 0.5
                    && p.iter().zip(q).any(|(x, y)| (x - y).abs() > 1e-6)
                {
                    return true;
                }
            }
        }
        false
    }

    fn diff_bbox(a: &[[f32; 4]], b: &[[f32; 4]], w: usize) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) = (w as i32, a.len() as i32 / w as i32, 0i32, 0i32);
        let mut any = false;
        for (i, (p, q)) in a.iter().zip(b).enumerate() {
            if p.iter().zip(q).all(|(x, y)| (x - y).abs() <= 1e-6) {
                continue;
            }
            any = true;
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
        if any { Rect::new(x0, y0, x1, y1) } else { Rect::EMPTY }
    }

    fn history_len(s: &Session) -> usize {
        s.active().unwrap().history.entries().len()
    }

    /// The exact public key list from the task specification, in order.
    const EXPECTED_KEYS: [&str; 68] = [
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
        "eyebrow_height",
        "eyebrow_distance",
        "eyebrow_length",
        "eyebrow_thickness",
        "eyebrow_arch",
        "eyebrow_tail",
        "eyebrow_tilt",
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
        "three_section_ratio",
        "face_symmetry",
        "eye_symmetry",
        "eyebrow_symmetry",
        "neck_slim",
        "neck_length",
        "e_line_adjustment",
        "midface_projection",
    ];

    #[test]
    fn the_parameter_table_has_the_exact_sixty_eight_spec_keys() {
        assert_eq!(FaceReshapeParams::COUNT, 68);
        assert_eq!(FaceReshapeParams::KEYS, EXPECTED_KEYS.as_slice(), "the public slider names and order must match the spec");
        assert_eq!(FaceReshapeParams::KEYS.first(), Some(&"head_scale"));
        assert_eq!(FaceReshapeParams::KEYS.last(), Some(&"midface_projection"));
        assert_eq!(ALL_FACE.len(), face_mesh::POINTS, "the global group covers every model landmark exactly once");
        assert_eq!(all_face_plus_derived().len(), face_mesh::POINTS + 6, "6 derived neck controls after the 478 model points");
        assert_eq!(IRIS_A.len() + IRIS_B.len(), 10);
        assert_eq!(NECK.len(), 6);
        let params = FaceReshapeParams::default();
        assert!(params.is_identity());
        for &key in FaceReshapeParams::KEYS {
            assert_eq!(params.get(key), Some(0.0), "{key}");
        }
    }

    #[test]
    fn parsing_clamps_out_of_range_values_and_rejects_bad_shapes() {
        let p = FaceReshapeParams::from_value(&json!({"head_scale": 250, "jaw_width": -250, "eye_size": 12.5})).unwrap();
        assert_eq!(p.head_scale, 100.0);
        assert_eq!(p.jaw_width, -100.0);
        assert_eq!(p.eye_size, 12.5);
        assert!(!p.is_identity());
        assert!(FaceReshapeParams::from_value(&json!({"nope": 1})).is_err(), "unknown key");
        assert!(FaceReshapeParams::from_value(&json!({"head_scale": "wide"})).is_err(), "wrong type");
        assert!(FaceReshapeParams::from_value(&json!([])).is_err(), "not an object");
        assert!(FaceReshapeParams::from_value(&json!({"head_scale": null})).is_err(), "null value");
        // Serde defaults are the same all-zero state.
        let from_serde: FaceReshapeParams = serde_json::from_value(json!({})).unwrap();
        assert!(from_serde.is_identity());
    }

    #[test]
    fn geometry_keeps_the_model_iris_and_derives_only_the_six_neck_controls() {
        let face = test_face();
        let g = Geometry::from_landmarks(&face.points, face.rect).unwrap();
        assert_eq!(g.base.len(), 484, "478 model points plus only 6 neck controls");
        assert_eq!(g.pts.len(), 484);
        let model: Vec<[f64; 2]> = face.points[468..478].iter().map(|p| [f64::from(p[0]), f64::from(p[1])]).collect();
        assert_eq!(g.base[468..478].to_vec(), model, "the iris controls are the model's own 468..478 points, not derived copies");
        assert!(Geometry::from_landmarks(&face.points[..468], face.rect).is_err(), "an old 468-point face is rejected");
    }

    #[test]
    fn every_parameter_moves_a_real_control_point() {
        let face = test_face();
        let mut g = Geometry::from_landmarks(&face.points, face.rect).unwrap();
        let base = g.pts.clone();
        let symmetry = ["face_symmetry", "eye_symmetry", "eyebrow_symmetry"];
        for &key in FaceReshapeParams::KEYS {
            g.pts.clone_from(&base);
            if symmetry.contains(&key) {
                // Symmetry on a perfectly symmetric face is legitimately a no-op; make the input
                // asymmetric first, as a real portrait usually is.
                for (i, p) in g.pts.iter_mut().enumerate() {
                    p[0] += if i % 2 == 0 { 2.0 } else { -2.0 };
                }
            }
            g.apply_one(key, 1.0);
            let moved = g.pts.iter().zip(&base).map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs())).fold(0.0f64, f64::max);
            assert!(moved > 1e-6, "`{key}` moved no control point");
        }
    }

    #[test]
    fn every_key_changes_at_least_one_output_pixel_at_plus_100() {
        for &key in FaceReshapeParams::KEYS {
            let mut s = session_with_face();
            let source = source_id(&s);
            let before = pixels(&s, source);
            let r = execute_with_face(&mut s, json!({"params": {key: 100}})).unwrap();
            assert_eq!(r["changed"], true, "`{key}` created no preview");
            let id = LayerId(r["layer"].as_u64().unwrap());
            let after = pixels(&s, id);
            let changed = before.iter().zip(&after).any(|(a, b)| a.iter().zip(b).any(|(x, y)| (x - y).abs() > 1e-6));
            assert!(changed, "`{key}` at +100 changed no output pixel");
            assert_eq!(pixels(&s, source), before, "`{key}` touched the source");
        }
    }

    #[test]
    fn preview_works_at_every_document_depth() {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut s = session_with_face_at(depth);
            let source = source_id(&s);
            let before = pixels(&s, source);
            let r = execute_with_face(&mut s, json!({"params": {"cheekbone_shrink": 40}})).unwrap();
            assert_eq!(r["changed"], true, "{depth:?}");
            let id = LayerId(r["layer"].as_u64().unwrap());
            assert!(!pixels(&s, id).is_empty(), "{depth:?}");
            assert_eq!(pixels(&s, source), before, "{depth:?}: source untouched");
        }
    }

    #[test]
    fn identity_params_change_nothing_and_make_no_layer() {
        let mut s = session_with_face();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let steps = history_len(&s);
        let r = execute_with_face(&mut s, json!({"params": {}})).unwrap();
        assert_eq!(r["changed"], false);
        assert_eq!(r["layer"], Value::Null);
        assert_eq!(r["points"], 478);
        assert!(names(&s).iter().all(|n| n != PREVIEW_NAME && n != FINAL_NAME));
        assert_eq!(pixels(&s, source), before, "the source layer is untouched");
        assert_eq!(history_len(&s), steps, "an identity call is not a history step");
    }

    #[test]
    fn a_slider_warps_only_the_face_region_above_the_source() {
        let mut s = session_with_face();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let face = test_face().rect;
        let r = execute_with_face(&mut s, json!({"params": {"jaw_width": 60}})).unwrap();
        assert_eq!(r["changed"], true);
        assert_eq!(r["name"], PREVIEW_NAME);
        assert_eq!(r["preview"], true);
        assert_eq!(r["points"], 478);
        assert_eq!(r["face"][0].as_f64(), Some(f64::from(face.x0)));
        assert_eq!(r["face"][1].as_f64(), Some(f64::from(face.y0)));
        assert_eq!(r["face"][2].as_f64(), Some(f64::from(face.width())));
        assert_eq!(r["face"][3].as_f64(), Some(f64::from(face.height())));
        let id = LayerId(r["layer"].as_u64().unwrap());
        let d = s.active().unwrap();
        assert_eq!(d.doc.layer(id).unwrap().name, PREVIEW_NAME);
        assert!(d.doc.layer(id).unwrap().visible);
        let order: Vec<_> = d.doc.walk().into_iter().map(|(_, _, l)| l.id).collect();
        let source_at = order.iter().position(|x| *x == source).unwrap();
        let preview_at = order.iter().position(|x| *x == id).unwrap();
        assert!(preview_at > source_at, "the preview sits above the source");
        assert_eq!(pixels(&s, source), before, "the source layer keeps its pixels");
        let after = pixels(&s, id);
        let diff = diff_bbox(&before, &after, 192);
        assert!(!diff.is_empty(), "the preview differs from the source");
        let region = face_region(face, d.doc.bounds());
        assert!(region.contains_rect(&diff), "the warp stays inside the face region: {diff:?} vs {region:?}");
        assert!(diff.y1 > face.y0 + (face.height() as f32 * 0.6) as i32, "a jaw slider reaches the lower face: {diff:?}");
        let content = d.doc.layer(id).unwrap().surface().unwrap().content_bounds();
        assert!(region.contains_rect(&content), "the preview stores only the face region: {content:?} vs {region:?}");
    }

    #[test]
    fn single_sliders_hit_their_intended_region() {
        // (slider, top limit as a fraction of the face box, expected to stay above it)
        let cases = [("eye_size", 0.55f32, true), ("nose_length", 0.75, true), ("upper_lip_thickness", 0.25, false)];
        for (key, fraction, above) in cases {
            let mut s = session_with_face();
            let source = source_id(&s);
            let before = pixels(&s, source);
            let face = test_face().rect;
            let r = execute_with_face(&mut s, json!({"params": {key: 80}})).unwrap();
            let id = LayerId(r["layer"].as_u64().unwrap());
            let diff = diff_bbox(&before, &pixels(&s, id), 192);
            assert!(!diff.is_empty(), "{key}: no pixels changed");
            let limit = face.y0 + (face.height() as f32 * fraction) as i32;
            if above {
                assert!(diff.y0 < limit, "{key}: changed {diff:?}, expected the upper face (face {face:?})");
            } else {
                assert!(diff.y0 > limit, "{key}: changed {diff:?}, expected the lower face (face {face:?})");
            }
        }
        // A global scale touches both sides of the face, not one feature.
        let mut s = session_with_face();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let face = test_face().rect;
        let r = execute_with_face(&mut s, json!({"params": {"head_scale": 80}})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let diff = diff_bbox(&before, &pixels(&s, id), 192);
        assert!(diff.width() > face.width() * 2 / 3, "head_scale spans the face: {diff:?}");
    }

    #[test]
    fn compare_hides_and_shows_the_preview_without_touching_the_source() {
        let mut s = session_with_face();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let r = execute_with_face(&mut s, json!({"params": {"eye_size": 40}})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        assert_eq!(history_len(&s), 2, "open + preview");

        let r = execute_with_face(&mut s, json!({"compare": true})).unwrap();
        assert_eq!(r["changed"], true);
        assert_eq!(r["compare"], true);
        assert_eq!(r["preview"], true);
        assert!(!s.active().unwrap().doc.layer(id).unwrap().visible, "compare hides the preview");
        assert_eq!(pixels(&s, source), before, "the source is untouched");

        let r = execute_with_face(&mut s, json!({"compare": false})).unwrap();
        assert_eq!(r["changed"], true);
        assert!(s.active().unwrap().doc.layer(id).unwrap().visible, "compare off shows it again");
        assert_eq!(pixels(&s, source), before);
    }

    #[test]
    fn apply_renames_the_preview_and_undo_removes_it() {
        let mut s = session_with_face();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let r = execute_with_face(&mut s, json!({"params": {"chin_length": 50}, "preview": true})).unwrap();
        let preview = LayerId(r["layer"].as_u64().unwrap());
        let r = execute_with_face(&mut s, json!({"params": {"chin_length": 50}, "preview": false})).unwrap();
        assert_eq!(r["name"], FINAL_NAME);
        assert_eq!(r["preview"], false);
        let final_id = LayerId(r["layer"].as_u64().unwrap());
        assert_eq!(final_id, preview, "Apply reuses the preview layer");
        let d = s.active().unwrap();
        assert!(d.doc.layer(final_id).is_some());
        assert!(d.doc.walk().iter().all(|(_, _, l)| l.name != PREVIEW_NAME), "no preview duplicate remains");
        assert_eq!(pixels(&s, source), before);
        s.execute("edit.undo", json!({})).unwrap();
        let d = s.active().unwrap();
        assert_eq!(d.doc.layer(final_id).unwrap().name, PREVIEW_NAME, "undo restores the preview");
        assert_eq!(pixels(&s, source), before);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(final_id).is_none(), "the next undo removes it");
        assert_eq!(pixels(&s, source), before);
    }

    #[test]
    fn apply_without_a_preview_creates_one_layer_and_one_history_step() {
        let mut s = session_with_face();
        let steps = history_len(&s);
        let r = execute_with_face(&mut s, json!({"params": {"jaw_width": 30}, "preview": false})).unwrap();
        assert_eq!(r["name"], FINAL_NAME);
        assert_eq!(r["preview"], false);
        let id = LayerId(r["layer"].as_u64().unwrap());
        assert!(s.active().unwrap().doc.layer(id).is_some());
        assert_eq!(history_len(&s), steps + 1, "one Apply is one history step");
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(id).is_none());
    }

    #[test]
    fn reset_removes_the_preview_and_is_idempotent() {
        let mut s = session_with_face();
        let r = execute_with_face(&mut s, json!({"params": {"nose_wing_width": 20}})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let r = execute_with_face(&mut s, json!({"reset": true})).unwrap();
        assert_eq!(r["changed"], true);
        assert_eq!(r["layer"], Value::Null);
        assert!(s.active().unwrap().doc.layer(id).is_none());
        let r = execute_with_face(&mut s, json!({"reset": true})).unwrap();
        assert_eq!(r["changed"], false);
    }

    #[test]
    fn a_coalesced_slider_drag_updates_one_layer_in_one_history_step() {
        let mut s = session_with_face();
        let steps = history_len(&s);
        let r = execute_with_face(&mut s, json!({"params": {"mouth_width": 20}, "coalesce": "face-drag"})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let r = execute_with_face(&mut s, json!({"params": {"mouth_width": 60}, "coalesce": "face-drag"})).unwrap();
        assert_eq!(LayerId(r["layer"].as_u64().unwrap()), id, "the same preview is updated");
        assert_eq!(history_len(&s), steps + 1, "the drag is one history step");
    }

    #[test]
    fn preview_layer_param_targets_that_layer_for_update_and_reset() {
        let mut s = session_with_face();
        let r = execute_with_face(&mut s, json!({"params": {"eyebrow_height": 20}})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let r = execute_with_face(&mut s, json!({"params": {"eyebrow_height": 50}, "previewLayer": id.0})).unwrap();
        assert_eq!(LayerId(r["layer"].as_u64().unwrap()), id);
        let r = execute_with_face(&mut s, json!({"reset": true, "previewLayer": id.0})).unwrap();
        assert_eq!(r["changed"], true);
        assert!(s.active().unwrap().doc.layer(id).is_none());
        let err = execute_with_face(&mut s, json!({"reset": true, "previewLayer": id.0})).unwrap_err().to_string();
        assert!(err.contains("no such layer"), "{err}");
    }

    #[test]
    fn all_zero_sliders_clear_a_stale_preview() {
        let mut s = session_with_face();
        execute_with_face(&mut s, json!({"params": {"head_scale": 20}})).unwrap();
        let r = execute_with_face(&mut s, json!({"params": {}})).unwrap();
        assert_eq!(r["changed"], true);
        assert!(names(&s).iter().all(|n| n != PREVIEW_NAME));
    }

    #[test]
    fn out_of_range_sliders_clamp_in_the_command() {
        for value in [1000, -1000] {
            let mut s = session_with_face();
            let r = execute_with_face(&mut s, json!({"params": {"jaw_width": value}})).unwrap();
            assert_eq!(r["changed"], true, "{value}");
            assert!(s.active().unwrap().doc.walk().iter().any(|(_, _, l)| l.name == PREVIEW_NAME));
        }
    }

    #[test]
    fn multi_face_selection_warps_only_the_chosen_faces_into_one_layer() {
        let faces = test_faces();
        let canvas = Rect::new(0, 0, 384, 192);
        let first = face_region(faces[0].rect, canvas);
        let second = face_region(faces[1].rect, canvas);
        assert!(first.x1 <= second.x0, "the test faces need disjoint regions: {first:?} {second:?}");

        // Default ("main") warps only the first (best) face and reports both detected.
        let mut s = session_with_two_faces();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let r = execute_with_faces(&mut s, json!({"params": {"jaw_width": 60}})).unwrap();
        assert_eq!(r["detected"], 2);
        assert_eq!(r["selected"], json!([1]));
        assert_eq!(r["faceMode"], "main");
        assert_eq!(r["faces"].as_array().map(Vec::len), Some(1));
        let id = LayerId(r["layer"].as_u64().unwrap());
        let after = pixels(&s, id);
        assert!(region_changed(&before, &after, 384, first), "the main face changed");
        assert!(!region_changed(&before, &after, 384, second), "the second face is untouched");
        assert_eq!(pixels(&s, source), before, "the source layer is untouched");
        assert_eq!(names(&s).iter().filter(|n| *n == PREVIEW_NAME).count(), 1, "one preview layer");

        // A single 1-based index addresses the detector's second face.
        let mut s = session_with_two_faces();
        let source = source_id(&s);
        let before = pixels(&s, source);
        let r = execute_with_faces(&mut s, json!({"params": {"jaw_width": 60}, "faces": [2]})).unwrap();
        assert_eq!(r["selected"], json!([2]));
        assert_eq!(r["faceMode"], "2");
        let id = LayerId(r["layer"].as_u64().unwrap());
        let after = pixels(&s, id);
        assert!(!region_changed(&before, &after, 384, first), "face 1 is untouched");
        assert!(region_changed(&before, &after, 384, second), "face 2 changed");
        assert_eq!(pixels(&s, source), before);

        // "all" and an explicit [1, 2] warp both faces in the same layer.
        for value in [json!("all"), json!([1, 2])] {
            let mut s = session_with_two_faces();
            let source = source_id(&s);
            let before = pixels(&s, source);
            let r = execute_with_faces(&mut s, json!({"params": {"jaw_width": 60}, "faces": value})).unwrap();
            assert_eq!(r["detected"], 2, "{value}");
            assert_eq!(r["selected"], json!([1, 2]), "{value}");
            assert_eq!(r["faces"].as_array().map(Vec::len), Some(2), "{value}");
            let id = LayerId(r["layer"].as_u64().unwrap());
            let after = pixels(&s, id);
            assert!(region_changed(&before, &after, 384, first), "{value}: face 1 changed");
            assert!(region_changed(&before, &after, 384, second), "{value}: face 2 changed");
            assert_eq!(pixels(&s, source), before, "{value}: source untouched");
            assert_eq!(names(&s).iter().filter(|n| *n == PREVIEW_NAME).count(), 1, "{value}: one preview layer");
        }
    }

    #[test]
    fn bad_face_choices_are_clear_errors_and_change_nothing() {
        let mut s = session_with_two_faces();
        let before = names(&s);
        for (value, expected) in [
            (json!([3]), "face 3 was not found; the detector found 2"),
            (json!([]), "at least one face"),
            (json!([0]), "1-based"),
            (json!([-1]), "1-based"),
            (json!([1.5]), "1-based"),
            (json!(["x"]), "1-based"),
            (json!({}), "`faces` must be"),
        ] {
            let err = execute_with_faces(&mut s, json!({"params": {"jaw_width": 20}, "faces": value})).unwrap_err().to_string();
            assert!(err.contains(expected), "{value}: {err}");
            assert_eq!(names(&s), before, "{value} changed the layers");
        }
        assert_eq!(execute_with_faces(&mut s, json!({"faces": "main", "params": {}})).unwrap()["changed"], false);
    }

    #[test]
    fn no_face_is_a_clear_error_and_changes_nothing() {
        let mut s = blank_session();
        let before = names(&s);
        for params in
            [json!({"params": {"head_scale": 20}}), json!({"params": {"head_scale": 20}, "faces": "all"}), json!({"params": {"head_scale": 20}, "faces": [1]})]
        {
            let err = s.execute(CMD, params).unwrap_err().to_string();
            assert!(err.contains("no face"), "{err}");
            assert_eq!(names(&s), before);
        }
    }
}

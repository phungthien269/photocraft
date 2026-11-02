//! MediaPipe 468-point face landmarks, the foundation for Face Reshape.
//!
//! The model is the MediaPipe Face Landmark Detector exported to ONNX (Apache-2.0; see
//! `data/face_landmark/NOTICE`). It takes one `1×3×192×192` RGB crop in `0..=1` and returns
//! normalized `x, y, z` for 468 points. [`landmarks`] builds that crop around a detected face
//! box, runs the model with `tract` and maps the points back to the coordinates of the image.
//!
//! The network is native only, like [`crate::faces`]: the browser build leaves `tract` and the
//! ~2.4 MB of weights out (the web build has a per-file size budget), so [`AVAILABLE`] is false
//! and [`landmarks`] returns an error there.
//!
//! The ONNX graph keeps its weights in the external file `face_landmark_detector.data`. Both
//! files are embedded in the binary; on first use they are copied to a unique directory under
//! the system temporary directory so `tract` can resolve the external data, and the model is
//! optimized and cached in a `OnceLock` for every later call. Nothing is written to the
//! document or the user's project.

/// Number of points the model returns.
pub const POINTS: usize = 468;

/// Whether this build can predict face landmarks (not the browser build: see the module docs).
pub const AVAILABLE: bool = cfg!(not(target_arch = "wasm32"));

/// Side of the square the model expects.
#[cfg(not(target_arch = "wasm32"))]
const INPUT_SIDE: usize = 192;

/// Padding around the face box: the crop's side is this multiple of the box's longest side.
#[cfg(not(target_arch = "wasm32"))]
const CROP_PADDING: f32 = 1.5;

/// 468 landmarks for the face in `face_rect` (`[x, y, w, h]`, document pixels).
///
/// `rgba` is straight RGBA `0..=1`, `width × height`, row-major. The crop is a square centred on
/// the face box with 1.5× its longest side, sampled bilinearly (edge-clamped when the square
/// reaches past the image), then resized to 192×192 and fed as NCHW RGB.
///
/// The returned `x` and `y` are in the same coordinates as `face_rect` (document pixels); `z`
/// is the model's relative depth, left unscaled so it can be compared between points. Every
/// coordinate is finite. Malformed buffers, zero-area or off-image faces and inference errors
/// return `Err`, never panic.
#[cfg(not(target_arch = "wasm32"))]
pub fn landmarks(rgba: &[[f32; 4]], width: usize, height: usize, face_rect: [f32; 4]) -> Result<Vec<[f32; 3]>, String> {
    if width == 0 || height == 0 {
        return Err("face landmarks need a non-empty image".into());
    }
    let expected = width.checked_mul(height).ok_or_else(|| "image is too large for face landmarks".to_string())?;
    if rgba.len() != expected {
        return Err(format!("face landmarks need {} RGBA pixels, got {}", expected, rgba.len()));
    }
    let crop = Crop::around(face_rect, width, height)?;
    let input = crop.input(rgba, width, height);
    crop.run(input)
}

/// The browser build has no model runtime (see the module docs): always an error.
#[cfg(target_arch = "wasm32")]
pub fn landmarks(_rgba: &[[f32; 4]], _width: usize, _height: usize, _face_rect: [f32; 4]) -> Result<Vec<[f32; 3]>, String> {
    Err("face landmarks aren't available in the browser version of PhotoCraft; use the desktop app".into())
}

/// The square mapped onto the image: `left/top` in pixels, `side` in pixels.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug)]
struct Crop {
    left: f32,
    top: f32,
    side: f32,
}

#[cfg(not(target_arch = "wasm32"))]
impl Crop {
    fn around(face_rect: [f32; 4], width: usize, height: usize) -> Result<Self, String> {
        let [x, y, w, h] = face_rect;
        if ![x, y, w, h].into_iter().all(f32::is_finite) {
            return Err("face box must have finite coordinates".into());
        }
        if w <= 0.0 || h <= 0.0 {
            return Err("face box has zero area".into());
        }
        let side = w.max(h) * CROP_PADDING;
        let (cx, cy) = (x + w * 0.5, y + h * 0.5);
        let (left, top) = (cx - side * 0.5, cy - side * 0.5);
        if !side.is_finite() || !left.is_finite() || !top.is_finite() || side <= 0.0 {
            return Err("face box is too large to crop".into());
        }
        let (iw, ih) = (width as f32, height as f32);
        if left >= iw || top >= ih || left + side <= 0.0 || top + side <= 0.0 {
            return Err("face box lies outside the image".into());
        }
        Ok(Self { left, top, side })
    }

    /// NCHW RGB `0..=1` for the 192×192 model input, bilinear and edge-clamped.
    fn input(self, rgba: &[[f32; 4]], width: usize, height: usize) -> Vec<f32> {
        let plane = INPUT_SIDE * INPUT_SIDE;
        let mut out = vec![0.0f32; plane * 3];
        let scale = self.side / INPUT_SIDE as f32;
        for ty in 0..INPUT_SIDE {
            let sy = self.top + (ty as f32 + 0.5) * scale - 0.5;
            for tx in 0..INPUT_SIDE {
                let sx = self.left + (tx as f32 + 0.5) * scale - 0.5;
                let [r, g, b] = self.sample(rgba, width, height, sx, sy);
                let i = ty * INPUT_SIDE + tx;
                out[i] = r;
                out[plane + i] = g;
                out[2 * plane + i] = b;
            }
        }
        out
    }

    /// Bilinear RGBA at a source pixel coordinate, clamped to the image, composited over white
    /// (like [`crate::faces::detect`], so transparent layer pixels don't turn into a black rim).
    fn sample(self, rgba: &[[f32; 4]], width: usize, height: usize, sx: f32, sy: f32) -> [f32; 3] {
        let (fx, fy) = (sx.floor(), sy.floor());
        let (tx, ty) = (sx - fx, sy - fy);
        let (x0, x1) = (clamp_index(fx, width), clamp_index(fx + 1.0, width));
        let (y0, y1) = (clamp_index(fy, height), clamp_index(fy + 1.0, height));
        let p00 = rgba[y0 * width + x0];
        let p10 = rgba[y0 * width + x1];
        let p01 = rgba[y1 * width + x0];
        let p11 = rgba[y1 * width + x1];
        let mut c = [0.0f32; 3];
        for k in 0..3 {
            let top = p00[k] * (1.0 - tx) + p10[k] * tx;
            let bottom = p01[k] * (1.0 - tx) + p11[k] * tx;
            let v = top * (1.0 - ty) + bottom * ty;
            let a = (p00[3] * (1.0 - tx) + p10[3] * tx) * (1.0 - ty) + (p01[3] * (1.0 - tx) + p11[3] * tx) * ty;
            let a = a.clamp(0.0, 1.0);
            c[k] = (v.clamp(0.0, 1.0) * a + (1.0 - a)).clamp(0.0, 1.0);
        }
        c
    }

    /// Run the cached model and map its normalized points back to the crop.
    #[cfg(not(target_arch = "wasm32"))]
    fn run(self, input: Vec<f32>) -> Result<Vec<[f32; 3]>, String> {
        use tract_onnx::prelude::*;

        let cached = MODEL.get_or_init(load_model);
        let model = cached.as_ref().map_err(|e| e.clone())?;
        let model = model.lock().map_err(|_| "face landmark model lock is poisoned".to_string())?;
        let array = tract_ndarray::Array4::from_shape_vec((1, 3, INPUT_SIDE, INPUT_SIDE), input).map_err(|e| format!("face landmark input: {e}"))?;
        let tensor: Tensor = array.into();
        let outputs = model.run(tvec!(tensor.into())).map_err(|e| format!("face landmark inference failed: {e}"))?;
        let mut found = None;
        for output in &outputs {
            let Ok(view) = output.to_plain_array_view::<f32>() else { continue };
            if view.len() != POINTS * 3 {
                continue;
            }
            let flat = view.as_slice().ok_or_else(|| "face landmark output is not contiguous".to_string())?;
            let mut points = Vec::with_capacity(POINTS);
            for i in 0..POINTS {
                let (nx, ny, z) = (flat[i * 3], flat[i * 3 + 1], flat[i * 3 + 2]);
                if !nx.is_finite() || !ny.is_finite() || !z.is_finite() {
                    return Err("face landmark model returned a non-finite point".into());
                }
                points.push([self.left + nx * self.side, self.top + ny * self.side, z]);
            }
            found = Some(points);
            break;
        }
        found.ok_or_else(|| "face landmark model returned no 468-point output".into())
    }
}

/// A pixel coordinate as a clamped index into an image of `len` pixels.
#[cfg(not(target_arch = "wasm32"))]
fn clamp_index(v: f32, len: usize) -> usize {
    // Float-to-int casts saturate in Rust; clamp again for the image size. `len` is never 0.
    (v as isize).clamp(0, len.saturating_sub(1) as isize) as usize
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use tract_onnx::prelude::*;

    const ONNX_NAME: &str = "face_landmark_detector.onnx";
    const DATA_NAME: &str = "face_landmark_detector.data";

    /// The graph and its weights; the external file must keep this exact name.
    const ONNX: &[u8] = include_bytes!("../data/face_landmark/face_landmark_detector.onnx");
    const DATA: &[u8] = include_bytes!("../data/face_landmark/face_landmark_detector.data");

    type Runnable = Arc<TypedRunnableModel>;

    /// Loaded once per process. `Err` keeps the reason, so later calls fail the same clear way
    /// instead of retrying a broken or unsupported model on every command.
    pub(super) static MODEL: OnceLock<Result<Mutex<Runnable>, String>> = OnceLock::new();

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    /// A unique temporary directory, removed when it goes out of scope.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir() -> Result<TempDir, String> {
        let base = std::env::temp_dir();
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for _ in 0..64 {
            let n = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("photocraft-face-landmark-{}-{nanos}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(TempDir(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("could not create a temporary directory for the face landmark model: {e}")),
            }
        }
        Err("could not create a unique temporary directory for the face landmark model".into())
    }

    /// Copy the embedded model and external data once, then build and optimize the graph.
    pub(super) fn load_model() -> Result<Mutex<Runnable>, String> {
        let dir = temp_dir()?;
        let onnx = dir.0.join(ONNX_NAME);
        let data = dir.0.join(DATA_NAME);
        write_file(&onnx, ONNX)?;
        write_file(&data, DATA)?;
        let model = load_from_dir(&onnx)?;
        Ok(Mutex::new(model))
    }

    fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
        fs::write(path, bytes).map_err(|e| format!("could not write the face landmark model to {}: {e}", path.display()))
    }

    fn load_from_dir(onnx: &Path) -> Result<Runnable, String> {
        tract_onnx::onnx()
            .model_for_path(onnx)
            .map_err(|e| format!("could not read the face landmark model: {e}"))?
            .into_optimized()
            .map_err(|e| format!("could not optimize the face landmark model: {e}"))?
            .into_runnable()
            .map_err(|e| format!("could not prepare the face landmark model: {e}"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
use native::{MODEL, load_model};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn synthetic(width: usize, height: usize) -> Vec<[f32; 4]> {
        let mut px = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                let v = ((x * 13 + y * 7) % 255) as f32 / 255.0;
                px.push([v, 1.0 - v, (v * 0.7 + 0.2).min(1.0), 1.0]);
            }
        }
        px
    }

    #[test]
    fn model_returns_468_finite_points_for_a_192_crop() {
        let px = synthetic(192, 192);
        let points = landmarks(&px, 192, 192, [48.0, 48.0, 96.0, 96.0]).expect("the bundled model must load and run");
        assert_eq!(points.len(), POINTS);
        assert!(points.iter().flatten().all(|v| v.is_finite()), "every x, y and z is finite");
    }

    #[test]
    fn crop_and_map_work_when_the_face_is_at_the_edge() {
        let (w, h) = (200, 150);
        let px = synthetic(w, h);
        for face in [[-12.0, 60.0, 48.0, 48.0], [176.0, 124.0, 24.0, 26.0], [-5.0, -8.0, 40.0, 36.0]] {
            let points = landmarks(&px, w, h, face).expect("edge crops must still run");
            assert_eq!(points.len(), POINTS);
            assert!(points.iter().flatten().all(|v| v.is_finite()));
            assert!(points.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max) > 2.0, "points are mapped to pixels, not left normalized");
        }
    }

    #[test]
    fn malformed_buffers_and_faces_are_errors() {
        assert!(landmarks(&[], 0, 0, [0.0, 0.0, 10.0, 10.0]).is_err());
        let px = synthetic(8, 8);
        assert!(landmarks(&px[..10], 8, 8, [0.0, 0.0, 4.0, 4.0]).is_err(), "wrong buffer length");
        assert!(landmarks(&px, 8, 8, [0.0, 0.0, 0.0, 4.0]).is_err(), "zero width");
        assert!(landmarks(&px, 8, 8, [0.0, 0.0, 4.0, 0.0]).is_err(), "zero height");
        assert!(landmarks(&px, 8, 8, [f32::NAN, 0.0, 4.0, 4.0]).is_err(), "NaN box");
        assert!(landmarks(&px, 8, 8, [100.0, 100.0, 4.0, 4.0]).is_err(), "outside the image");
    }
}

//! Window › Color Presets: named colour/tone looks captured from a document's visible
//! adjustment layers and applied as a new pass-through group.
//!
//! A preset keeps whole-image colour work only: the adjustment kind, the source layer name, the
//! complete parameter set (`crate::adjust_params::to_params`), layer opacity and blend mode. A
//! Color Lookup also keeps its whole LUT table, so a saved or imported look stays self-contained
//! when the source file moves. Masks, painted pixels, effects, groups and every other layer kind
//! are not part of a look.
//! Applying a preset never edits existing layers: it inserts a new top-level group named
//! `Preset • <name>` whose children are fresh adjustment layers, in one undo step.
//!
//! Built-in looks are our own (clean-room), each a single Basic Color layer.

use std::sync::Arc;

use photocraft_color::{BlendMode, ColorMode};
use photocraft_doc::{Adjustment, Layer, LayerContent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Group, Named, always, bad, edit_groups, find, group_index, has_doc, req_str, str_param, unique_name};
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// Built-in group holding the five starter looks.
pub const BUILTIN_GROUP: &str = "PhotoCraft";
/// Group a capture lands in when the caller names no group.
pub const USER_GROUP: &str = "My Presets";
/// Group an imported LUT lands in when the caller names no group.
pub const LUT_GROUP: &str = "LUTs";

/// A whole colour/tone look: adjustment layers in compositing order, bottom-to-top.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorPreset {
    pub name: String,
    pub layers: Vec<ColorPresetLayer>,
}

/// One adjustment layer of a preset. Only whole-image colour data is kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorPresetLayer {
    /// Adjustment kind id, as `crate::commands::adjustment_kind` writes it.
    pub kind: String,
    /// Source layer name; empty means the kind's display label when applied.
    pub name: String,
    /// Complete `crate::adjust_params::to_params` payload.
    pub params: Value,
    /// Layer opacity, clamped to `0..=1` when applied.
    pub opacity: f32,
    /// `BlendMode` serde name (`"Normal"`, `"Multiply"`, …).
    pub blend: String,
    /// A self-contained Color Lookup table, so the look keeps working when its source file moves.
    /// `None` for every other kind and for presets saved before LUTs were kept.
    pub lut: Option<ColorPresetLut>,
}

/// A LUT stored inside a preset: the whole table, independent of the file it came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorPresetLut {
    /// The look label (the LUT file's name, as Color Lookup shows it).
    pub name: String,
    /// Edge length of the cubic table.
    pub size: u32,
    /// `size`³ × 3 red-fastest RGB values in 0..=1 (`((b·size + g)·size + r)·3`).
    pub data: Vec<f32>,
    /// Tetrahedral instead of trilinear interpolation.
    pub tetrahedral: bool,
    /// Ordered dither to hide banding.
    pub dither: bool,
}

impl Default for ColorPresetLayer {
    fn default() -> Self {
        ColorPresetLayer { kind: String::new(), name: String::new(), params: json!({}), opacity: 1.0, blend: "Normal".into(), lut: None }
    }
}

impl Named for ColorPreset {
    fn name(&self) -> &str {
        &self.name
    }

    fn set_name(&mut self, n: String) {
        self.name = n;
    }
}

impl ColorPreset {
    /// The list/result JSON shape: `{name, layers:[{kind,name,params,opacity,blend,lut}]}`.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "layers": self.layers.iter().map(ColorPresetLayer::to_json).collect::<Vec<_>>(),
        })
    }
}

impl ColorPresetLayer {
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind,
            "name": self.name,
            "params": self.params,
            "opacity": self.opacity,
            "blend": self.blend,
            "lut": self.lut,
        })
    }

    /// Rebuild this layer's adjustment. A stored LUT is used as-is, never re-read from disk.
    pub fn adjustment(&self, mode: ColorMode) -> Result<Adjustment> {
        if let Some(lut) = &self.lut {
            if self.kind.trim() != "colorLookup" {
                return Err(bad("color.presets", "an embedded LUT only belongs to a colorLookup layer"));
            }
            return lut.adjustment(&self.name);
        }
        crate::adjust_params::from_params(self.kind.trim(), &self.params, None, mode)
    }
}

impl ColorPresetLut {
    /// A Color Lookup adjustment carrying this exact table. The size and data length are checked
    /// here, so a damaged inline preset fails before the document changes.
    fn adjustment(&self, layer_name: &str) -> Result<Adjustment> {
        const CMD: &str = "color.presets";
        let size = self.size as usize;
        if !(2..=photocraft_cms::lutfile::MAX_SIZE).contains(&size) {
            return Err(bad(CMD, format!("unsupported embedded LUT size {}", self.size)));
        }
        let expected =
            size.checked_pow(3).and_then(|entries| entries.checked_mul(3)).ok_or_else(|| bad(CMD, format!("unsupported embedded LUT size {}", self.size)))?;
        if self.data.len() != expected {
            return Err(bad(CMD, format!("embedded LUT size {size} needs {expected} values, found {}", self.data.len())));
        }
        if !self.data.iter().all(|v| v.is_finite()) {
            return Err(bad(CMD, "embedded LUT has a non-finite value"));
        }
        let name = if !self.name.trim().is_empty() {
            self.name.trim().to_string()
        } else if !layer_name.trim().is_empty() {
            layer_name.trim().to_string()
        } else {
            "Color Lookup".into()
        };
        Ok(Adjustment::ColorLookup { name, lut: Some(Arc::new(self.data.clone())), size: self.size, tetrahedral: self.tetrahedral, dither: self.dither })
    }
}

/// The adjustment a preset layer's list JSON rebuilds, using an embedded LUT when it is sound.
/// `None` when the layer itself cannot be recreated (unknown kind, bad params); a damaged LUT
/// falls back to the parameter-only form, so a card preview still draws.
pub fn adjustment_from_layer(value: &Value, mode: ColorMode) -> Option<Adjustment> {
    let layer: ColorPresetLayer = serde_json::from_value(value.clone()).ok()?;
    if layer.kind.trim().is_empty() {
        return None;
    }
    if layer.lut.is_some()
        && let Ok(adjustment) = layer.adjustment(mode)
    {
        return Some(adjustment);
    }
    crate::adjust_params::from_params(layer.kind.trim(), &layer.params, None, mode).ok()
}

/// The five starter looks, all our own. Each is one Basic Color layer at full opacity, Normal.
pub fn builtin() -> Vec<Group<ColorPreset>> {
    let look = |name: &str, params: Value| ColorPreset {
        name: name.into(),
        layers: vec![ColorPresetLayer { kind: "basicColor".into(), name: "Basic Color".into(), params, opacity: 1.0, blend: "Normal".into(), lut: None }],
    };
    vec![Group::new(
        BUILTIN_GROUP,
        vec![
            look(
                "Warm Portrait",
                json!({
                    "temperature": 25.0, "tint": -5.0, "exposure": 0.15, "contrast": 10.0, "highlights": -15.0,
                    "shadows": 15.0, "whites": 0.0, "blacks": 0.0, "vibrance": 15.0, "saturation": 0.0,
                }),
            ),
            look(
                "Cool Soft",
                json!({
                    "temperature": -20.0, "tint": 5.0, "exposure": 0.05, "contrast": -5.0, "highlights": -10.0,
                    "shadows": 10.0, "whites": 0.0, "blacks": 5.0, "vibrance": 0.0, "saturation": -5.0,
                }),
            ),
            look(
                "Punchy",
                json!({
                    "temperature": 5.0, "tint": 0.0, "exposure": 0.0, "contrast": 25.0, "highlights": -10.0,
                    "shadows": 5.0, "whites": 10.0, "blacks": -10.0, "vibrance": 25.0, "saturation": 5.0,
                }),
            ),
            look(
                "Faded Film",
                json!({
                    "temperature": 10.0, "tint": 3.0, "exposure": 0.0, "contrast": -15.0, "highlights": -20.0,
                    "shadows": 20.0, "whites": -5.0, "blacks": 15.0, "vibrance": -5.0, "saturation": -15.0,
                }),
            ),
            look(
                "Black & White",
                json!({
                    "temperature": 0.0, "tint": 0.0, "exposure": 0.0, "contrast": 15.0, "highlights": 0.0,
                    "shadows": 0.0, "whites": 0.0, "blacks": -5.0, "vibrance": 0.0, "saturation": -100.0,
                }),
            ),
        ],
    )]
}

/// Read the saved set leniently: a damaged group or preset is dropped without discarding the
/// rest. `None` when the value is not a list at all, so a malformed set keeps the defaults.
pub(crate) fn load(value: Value) -> Option<Vec<Group<ColorPreset>>> {
    let groups = value.as_array()?;
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let Some(name) = group.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()) else { continue };
        let Some(items) = group.get("items").and_then(Value::as_array) else { continue };
        let mut presets = Vec::with_capacity(items.len());
        for item in items {
            let Ok(mut preset) = serde_json::from_value::<ColorPreset>(item.clone()) else { continue };
            let name = preset.name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            preset.name = name;
            presets.push(preset);
        }
        out.push(Group::new(name, presets));
    }
    Some(out)
}

/// A blend mode as its serde name, the form `ColorPresetLayer.blend` stores.
fn blend_name(blend: BlendMode) -> String {
    serde_json::to_value(blend).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "Normal".into())
}

/// The next free automatic name: `Look 1`, `Look 2`, ….
fn next_look_name(groups: &[Group<ColorPreset>]) -> String {
    (1..).map(|i| format!("Look {i}")).find(|n| find(groups, n, None).is_none()).unwrap_or_else(|| "Look 1".into())
}

/// Validate every layer before the document changes; `Err` leaves no partial stack.
fn ready_layers(preset: &ColorPreset, mode: ColorMode, cmd: &str) -> Result<Vec<ReadyLayer>> {
    if preset.layers.is_empty() {
        return Err(bad(cmd, format!("preset \"{}\" has no layers", preset.name)));
    }
    preset
        .layers
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            let n = i + 1;
            let kind = layer.kind.trim();
            if kind.is_empty() {
                return Err(bad(cmd, format!("layer {n} has no adjustment kind")));
            }
            let adjustment = layer.adjustment(mode).map_err(|e| bad(cmd, format!("layer {n} ({kind}): {e}")))?;
            if !layer.opacity.is_finite() {
                return Err(bad(cmd, format!("layer {n} ({kind}): opacity is not a number")));
            }
            let blend = crate::commands::blend_from_str(layer.blend.trim())
                .filter(|b| *b != BlendMode::PassThrough)
                .ok_or_else(|| bad(cmd, format!("layer {n} ({kind}): unknown blend mode \"{}\"", layer.blend)))?;
            let name = if layer.name.trim().is_empty() { adjustment.label().to_string() } else { layer.name.trim().to_string() };
            Ok(ReadyLayer { name, adjustment, opacity: layer.opacity.clamp(0.0, 1.0), blend })
        })
        .collect()
}

struct ReadyLayer {
    name: String,
    adjustment: Adjustment,
    opacity: f32,
    blend: BlendMode,
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let groups: Vec<Value> =
        s.presets.color_presets.iter().map(|g| json!({"name": g.name, "presets": g.items.iter().map(ColorPreset::to_json).collect::<Vec<_>>()})).collect();
    Ok(json!({"groups": groups}))
}

fn new_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "color.presets.new";
    let mut layers = Vec::new();
    let mut skipped = 0usize;
    {
        let d = s.active().ok_or(EngineError::NoDocument)?;
        for (path, _, layer) in d.doc.walk() {
            // A hidden layer, or anything inside a hidden group, is not part of the look.
            if !d.doc.shown_at(&path) {
                continue;
            }
            let LayerContent::Adjustment(adjustment) = &layer.content else { continue };
            // Unsupported adjustments only exist as imported data; there is no parameter set to
            // recreate them from, so they cannot be saved into a preset.
            if matches!(adjustment, Adjustment::Unsupported { .. }) {
                skipped += 1;
                continue;
            }
            layers.push(ColorPresetLayer {
                kind: crate::commands::adjustment_kind(adjustment).to_string(),
                name: layer.name.clone(),
                params: crate::adjust_params::to_params(adjustment),
                opacity: layer.opacity.clamp(0.0, 1.0),
                blend: blend_name(layer.blend),
                // A Color Lookup keeps its whole table: the parameters alone would rebuild an
                // identity lookup once the source file is gone.
                lut: match adjustment {
                    Adjustment::ColorLookup { name, lut: Some(data), size, tetrahedral, dither } => {
                        Some(ColorPresetLut { name: name.clone(), size: *size, data: data.as_ref().clone(), tetrahedral: *tetrahedral, dither: *dither })
                    }
                    _ => None,
                },
            });
        }
    }
    if layers.is_empty() {
        return Err(bad(CMD, "no visible adjustment layers to save"));
    }
    let gi = group_index(&mut s.presets.color_presets, Some(str_param(p, "group").unwrap_or(USER_GROUP)));
    let name = match str_param(p, "name") {
        // Names are unique across all groups (agents address presets by name); `apply`'s optional
        // `group` still narrows where older data holds a duplicate.
        Some(name) => unique_name(&s.presets.color_presets, name),
        None => next_look_name(&s.presets.color_presets),
    };
    s.presets.color_presets[gi].items.push(ColorPreset { name: name.clone(), layers });
    let group = s.presets.color_presets[gi].name.clone();
    let count = s.presets.color_presets[gi].items.last().map_or(0, |p| p.layers.len());
    s.presets_changed();
    Ok(json!({"name": name, "group": group, "layers": count, "skipped": skipped}))
}

/// Image › Adjustments › Color Lookup folder import: one self-contained preset per LUT file.
/// Native only: the web build has no file system.
fn import_lut_folder(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "color.presets.importLutFolder";
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (s, p);
        Err(bad(CMD, "importing a LUT folder is not available on the web (no file system)"))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        import_lut_folder_native(s, p, CMD)
    }
}

/// Subfolders below the import folder are walked at most this deep.
#[cfg(not(target_arch = "wasm32"))]
const MAX_LUT_DEPTH: usize = 32;
/// An import lists at most this many LUT files; a larger tree is an error, not a partway import.
#[cfg(not(target_arch = "wasm32"))]
const MAX_LUT_FILES: usize = 4096;
/// The LUT files an import holds at once, so a folder of very large tables cannot exhaust memory.
#[cfg(not(target_arch = "wasm32"))]
const MAX_LUT_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(not(target_arch = "wasm32"))]
fn import_lut_folder_native(s: &mut Session, p: &Value, cmd: &str) -> Result<Value> {
    let path = req_str(p, "path", cmd)?.to_string();
    let group = match p.get("group") {
        None | Some(Value::Null) => LUT_GROUP.to_string(),
        Some(Value::String(name)) if !name.trim().is_empty() => name.trim().to_string(),
        Some(Value::String(_)) => LUT_GROUP.to_string(),
        Some(_) => return Err(bad(cmd, "`group` must be a string")),
    };
    let recurse = match p.get("includeSubfolders") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`includeSubfolders` must be true or false")),
    };
    let root = std::path::Path::new(&path);
    if !root.is_dir() {
        return Err(bad(cmd, format!("{path}: not a folder")));
    }
    // Read and parse everything before touching the library, so a failed walk adds nothing.
    let candidates = lut_files(root, recurse, cmd)?;
    let mut parsed = Vec::new();
    let mut errors = Vec::new();
    let mut total_bytes = 0u64;
    for (shown, file) in candidates {
        total_bytes = total_bytes.saturating_add(std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0));
        if total_bytes > MAX_LUT_BYTES {
            return Err(bad(cmd, format!("the folder's LUT files exceed {} MiB", MAX_LUT_BYTES / (1024 * 1024))));
        }
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) => {
                errors.push(json!({"file": shown, "error": format!("cannot read: {e}")}));
                continue;
            }
        };
        let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or(&shown).to_string();
        let lut = match photocraft_cms::lutfile::parse(&file_name, &bytes) {
            Ok(lut) => lut,
            Err(e) => {
                errors.push(json!({"file": shown, "error": e.0}));
                continue;
            }
        };
        // Color Lookup applies the plain 0..1 domain; a custom one would silently shift the look.
        if !lut.domain_is_default() {
            errors.push(json!({
                "file": shown,
                "error": format!("DOMAIN_MIN {:?} / DOMAIN_MAX {:?} is not supported yet", lut.domain_min, lut.domain_max),
            }));
            continue;
        }
        let stem = file.file_stem().and_then(|n| n.to_str()).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).unwrap_or_else(|| file_name.clone());
        parsed.push((stem.clone(), ColorPresetLut { name: stem, size: lut.size as u32, data: lut.data, tetrahedral: false, dither: false }));
    }
    let imported = parsed.len();
    let skipped = errors.len();
    if imported > 0 {
        let gi = group_index(&mut s.presets.color_presets, Some(&group));
        for (stem, lut) in parsed {
            // Never overwrite an existing preset: the next free suffix keeps names addressable.
            let name = unique_name(&s.presets.color_presets, &stem);
            s.presets.color_presets[gi].items.push(ColorPreset {
                name,
                layers: vec![ColorPresetLayer {
                    kind: "colorLookup".into(),
                    name: String::new(),
                    params: json!({"interpolation": "trilinear", "dither": false}),
                    opacity: 1.0,
                    blend: "Normal".into(),
                    lut: Some(lut),
                }],
            });
        }
        s.presets_changed();
    }
    Ok(json!({"group": group, "imported": imported, "skipped": skipped, "errors": errors}))
}

/// The LUT files under `root`, as `(path relative to root, full path)` sorted by name. With
/// `recurse` the tree is walked depth first to [`MAX_LUT_DEPTH`], each folder once (symlink loops
/// are not followed twice), and more than [`MAX_LUT_FILES`] is an error.
#[cfg(not(target_arch = "wasm32"))]
fn lut_files(root: &std::path::Path, recurse: bool, cmd: &str) -> Result<Vec<(String, std::path::PathBuf)>> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn is_lut(path: &std::path::Path) -> bool {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "cube" | "3dl" | "look"))
    }

    let mut seen = HashSet::new();
    if let Ok(canonical) = std::fs::canonicalize(root) {
        seen.insert(canonical);
    }
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    while let Some((folder, depth)) = stack.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            // The top folder must be readable; an unreadable subfolder is skipped.
            Err(e) if depth == 0 => return Err(bad(cmd, format!("{}: {e}", folder.display()))),
            Err(_) => continue,
        };
        let mut files = Vec::new();
        let mut subs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if recurse {
                    subs.push(path);
                }
            } else if is_lut(&path) {
                files.push(path);
            }
        }
        files.sort();
        for file in files {
            if out.len() >= MAX_LUT_FILES {
                return Err(bad(cmd, format!("more than {MAX_LUT_FILES} LUT files")));
            }
            let shown = file.strip_prefix(root).unwrap_or(&file).to_string_lossy().replace('\\', "/");
            out.push((shown, file));
        }
        if !recurse || depth >= MAX_LUT_DEPTH {
            continue;
        }
        subs.sort();
        // Pushed in reverse so the stack pops them in name order.
        for sub in subs.into_iter().rev() {
            let Ok(canonical) = std::fs::canonicalize(&sub) else { continue };
            if seen.insert(canonical) {
                stack.push((sub, depth + 1));
            }
        }
    }
    Ok(out)
}

/// The preset named by `"preset"`: a library name (optionally narrowed by `"group"`) or an
/// inline `{name, layers}` object for automation and tests.
fn preset_param(s: &Session, p: &Value, cmd: &str) -> Result<ColorPreset> {
    match p.get("preset") {
        Some(Value::String(name)) => {
            let name = name.trim();
            if name.is_empty() {
                return Err(bad(cmd, "missing `preset`"));
            }
            let (gi, ii) = find(&s.presets.color_presets, name, str_param(p, "group"))
                .ok_or_else(|| bad(cmd, format!("no color preset \"{name}\" (see color.presets.list)")))?;
            Ok(s.presets.color_presets[gi].items[ii].clone())
        }
        Some(v @ Value::Object(_)) => {
            let mut preset: ColorPreset = serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("bad inline preset: {e}")))?;
            let name = preset.name.trim().to_string();
            if name.is_empty() {
                return Err(bad(cmd, "an inline preset needs a name"));
            }
            preset.name = name;
            Ok(preset)
        }
        Some(_) => Err(bad(cmd, "`preset` is a name or {name,layers}")),
        None => Err(bad(cmd, "missing `preset`")),
    }
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "color.presets.apply";
    let preset = preset_param(s, p, CMD)?;
    let mode = s.active().ok_or(EngineError::NoDocument)?.doc.mode;
    let ready = ready_layers(&preset, mode, CMD)?;
    let n = ready.len();
    let group_name = format!("Preset \u{2022} {}", preset.name);
    let label = format!("Apply Preset \u{201c}{}\u{201d}", preset.name);
    let (gid, ids) = s.edit(&label, |doc, active| {
        let mut children = Vec::with_capacity(n);
        let mut ids = Vec::with_capacity(n);
        for layer in ready {
            let mut l = Layer::new(layer.name, LayerContent::Adjustment(layer.adjustment));
            l.opacity = layer.opacity;
            l.blend = layer.blend;
            ids.push(l.id);
            children.push(l);
        }
        // `Layer::group` uses Pass Through, so the adjustment layers reach the composite below.
        let gid = doc.insert_above(None, Layer::group(group_name.clone(), children));
        *active = Some(gid);
        Ok((gid, ids))
    })?;
    crate::layer_multi_cmds::reselect(s, vec![gid], Some(gid));
    Ok(json!({
        "name": preset.name,
        "group": gid.0,
        "layers": ids.iter().map(|id| id.0).collect::<Vec<_>>(),
    }))
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "color.presets.edit";
    let action = req_str(p, "action", CMD)?.to_string();
    let r = edit_groups(&mut s.presets.color_presets, &action, p, CMD)?;
    s.presets_changed();
    Ok(r)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "color.presets.list",
            label: "Color Presets",
            menu: &[],
            shortcut: None,
            params: "{} → {groups:[{name,presets:[{name,layers:[{kind,name,params,opacity,blend,lut}]}]}]} (lut is null or {name,size,data,tetrahedral,dither}, a self-contained Color Lookup table)",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "color.presets.new",
            label: "New Color Preset…",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str? (default a unique "Look 1", "Look 2"…),"group":name?="My Presets" (created when missing)} → {name,group,layers,skipped} (captures every visible adjustment layer, bottom-to-top, with opacity and blend; unsupported imported adjustments have no parameter set and are counted as skipped; no masks, effects or groups)"##,
            enabled: has_doc,
            run: new_preset,
            journal: true,
        },
        CommandSpec {
            id: "color.presets.apply",
            label: "Apply Color Preset",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name|{name,layers:[{kind,name,params,opacity,blend,lut}]},"group":name? (narrows a library lookup)} → {name,group,layers} (one new top-level Pass Through group at the top, one undo step; existing layers are never modified; an embedded lut rebuilds its Color Lookup table directly)"##,
            enabled: has_doc,
            run: apply,
            journal: true,
        },
        CommandSpec {
            id: "color.presets.importLutFolder",
            label: "Import LUT folder…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":folder,"group":name?="LUTs" (created when missing),"includeSubfolders":bool=false (walk at most 32 levels and 4096 files)} → {group,imported,skipped,errors:[{file,error}]} (one preset per .cube/.3dl/.look file, named from the file stem and kept unique across the library; each is one Color Lookup layer with a self-contained copy of the table; damaged files, custom input domains and unreadable files are reported and skipped; native only)"##,
            enabled: always,
            run: import_lut_folder,
            journal: true,
        },
        CommandSpec {
            id: "color.presets.edit",
            label: "Edit Color Presets",
            menu: &[],
            shortcut: None,
            params: super::GROUP_EDIT_PARAMS,
            enabled: always,
            run: edit,
            journal: true,
        },
    ]
}

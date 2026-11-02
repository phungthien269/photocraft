# MediaPipe Face Landmarker 478 (ONNX)

`face_landmark_detector.onnx` is Google MediaPipe's **Face Landmarker 478-point** face
landmark model, exported to ONNX (the graph records `tf2onnx 1.17.0` as its producer). The
export is self-contained: all weights are stored inside the ONNX file, with no external data
file. It is used by `crates/algo/src/face_mesh.rs` for Face Reshape and the read-only
`retouchAI.faceLandmarks` query.

## Contract

| Tensor | Shape | Meaning |
|---|---|---|
| input `input_12` | `1 × 256 × 256 × 3`, float32 | NHWC RGB face crop, values `0..=1` |
| output `Identity` | `1 × 1 × 1 × 1434` | 478 landmarks as `x, y, z`, x/y in crop pixels `0..=256` |
| output `Identity_1` | `1 × 1 × 1 × 1` | face-presence logit |
| output `Identity_2` | `1` | face-presence probability |

Points `0..468` are the standard MediaPipe face mesh; `468..478` are the two irises (five
points per eye: centre, right, top, left, bottom). `z` is the model's relative depth.

## Source and licence

- Upstream model: [MediaPipe Face Landmarker](https://ai.google.dev/edge/mediapipe/solutions/vision/face_landmarker),
  Google MediaPipe, Apache License 2.0.
- Upstream weights: the Face Landmarker task bundle (`face_landmarker.task`, float16 v1,
  `face_landmarks_detector.tflite`, sha256
  `64184e229b263107bc2b804c6625db1341ff2bb731874b0bcc2fe6544e0bc9ff`), published at
  <https://storage.googleapis.com/mediapipe-models/face_landmarker/face_landmarker/float16/1/face_landmarker.task>.
- Upstream code and model card: <https://github.com/google-ai-edge/mediapipe>.
- ONNX conversion: tf2onnx 1.17.0, IR version 8.

Bundled file: 4,920,995 bytes, sha256
`f38c3321ceffbc9e95103480ad38cc3f52e7e1bde2bcee7cd9355d0b9138ac0c`.

Licensed under the Apache License, Version 2.0; see [`LICENSE-APACHE`](LICENSE-APACHE) and
[`NOTICE`](NOTICE) in this directory.

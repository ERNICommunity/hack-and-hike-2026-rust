# The models the firmware carries

These files are the neural networks and the impostor bank, in the FKB1
format (`crates/vision/src/blob.rs`). The application includes them with
`include_bytes!`, so they end up in the board's flash. At start it copies
them into PSRAM, which reads about four times as fast as flash, and runs
the networks from there.

| File | What | Size |
| --- | --- | --- |
| `mfn_s8_v1.fkb` | the face recognizer: Espressif's MFN_S8_V1, `i8` with a power-of-two scale per tensor as Espressif published it, imported by `facekit import-espdl` | 1.26 MB |
| `yunet.int8.fkb` | the face detector: YuNet, `i8` weights with per-channel scales and the calibrated 16-bit activation mappings | 92 KB |
| `impostors.fkb` | 200 embeddings of strangers, `i8` with one scale: what recognition compares against so that a drifting score cannot let the wrong person in | 102 KB |
| `*.txt` | one line per tensor of the file beside it: name, type, layout, shape | |

They are generated, not written by hand. `tools/facekit/README.md` has
the commands; `crates/vision/tests/fixtures/README.md` repeats them with
the exact paths, and `docs/face_id/README.md` explains what the numbers in
them mean and what they cost in accuracy.

Licences differ from the rest of this repository and follow the models
they come from, both MIT:

- MFN_S8_V1: Copyright (c) 2021 Espressif Systems (Shanghai) Co., Ltd.,
  from ESP-DL (`models/human_face_recognition`):
  [`LICENSE-MFN_S8_V1`](LICENSE-MFN_S8_V1).
- YuNet: Copyright (c) 2020 Shiqi Yu, from OpenCV Zoo
  (`models/face_detection_yunet`): [`LICENSE-YuNet`](LICENSE-YuNet).

The impostor bank holds embeddings that MFN_S8_V1 computed from photos
of Labeled Faces in the Wild; the photos themselves are not in the
repository.

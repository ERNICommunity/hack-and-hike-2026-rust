# The models the firmware carries

These files are the neural networks and the impostor bank, in the FKB1
format (`crates/vision/src/blob.rs`). The application includes them with
`include_bytes!`, so they end up in the board's flash. At start it copies
the two networks into PSRAM (1.5 MB), arranged for the vector kernels;
the impostor bank is read from flash.

| File | What | Size |
| --- | --- | --- |
| `edgeface_xxs.int8.fkb` | the face recognizer: EdgeFace-XXS, `i8` weights with per-channel scales, folded LayerNorms and the calibrated 16-bit activation mappings | 1.37 MB |
| `yunet.int8.fkb` | the face detector: YuNet, likewise | 92 KB |
| `impostors.fkb` | 200 embeddings of strangers, `i8` with one scale: what recognition compares against so that a drifting score cannot let the wrong person in | 102 KB |
| `*.txt` | one line per tensor of the file beside it: name, type, layout, shape | |

They are generated, not written by hand. `tools/facekit/README.md` has
the commands; `crates/vision/tests/fixtures/README.md` repeats them with
the exact paths, and `docs/face_id/README.md` explains what the numbers in
them mean and what they cost in accuracy.

Licences differ from the rest of this repository and follow the models
they come from: EdgeFace-XXS is **CC BY-NC-SA 4.0** (Idiap Research
Institute), YuNet is **MIT** (OpenCV Zoo). The impostor bank is derived
from Labeled Faces in the Wild.

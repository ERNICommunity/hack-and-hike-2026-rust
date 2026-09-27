# facekit

The developer-machine side of the face ID application. It runs on your
computer, never on the board, and it is plain Rust: no Python anywhere.

It turns the downloaded neural-network models into data the firmware can
use, and it measures how well the whole pipeline works, using the same
`crates/vision` code that runs on the board.

## Build and run

facekit is a workspace of its own, built with the stable toolchain from
`rust-toolchain.toml` in this folder. Always run Cargo from this folder:
the repository root's Cargo configuration builds for the ESP32-S3, and
this folder's `.cargo/config.toml` overrides that with the host target.

```bash
cd tools/facekit
cargo run --release -- inspect --input-shape 1,3,112,112 models/edgeface_xxs.onnx
cargo run --release -- inspect --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx
```

The model files are in `models/`; [`models/README.md`](models/README.md)
says where they come from and under which licences.

## Commands

| Command | Step | What it does |
| --- | --- | --- |
| `inspect <model.onnx> [--strict] [--input-shape ...]` | 1 | Lists operators, weights and shapes; checks every operator against the list the firmware implements; loads the model through tract. The results for the two models are in [`docs/face_id/`](../../docs/face_id/). |
| `export <model.onnx> <out.fkb> --input-shape ...` | 3 | Writes every weight as an FKB1 file in the layouts the firmware's kernels want (`src/export.rs` has the table), renames unnamed weights after their module path, folds run-time constants (EdgeFace's positional encoding), and writes a `.txt` listing next to the file. |
| `golden <edgeface\|yunet> <model.onnx> <out.fkb> [--image photo.jpg] [--source face\|noise]` | 3 | Runs the model in tract on a photo or a synthetic image and writes the input, every block boundary and the outputs as an FKB1 file, for the firmware tests. |

| `crops <yunet.onnx> <photos> <out>` | 6 | Finds the faces in a folder of photos (YuNet in tract at the photo's size, then the firmware's own decoding and alignment) and writes `out/crops/` (112x112 aligned crops) and `out/frames/` (320x240 frames with the face filling the height). |
| `quantize <edgeface\|yunet> <f32.fkb> <samples> <out.fkb> [--limit N]` | 6 | Calibrates on the samples (crops or frames), quantizes the weights, folds the LayerNorms, writes the integer file, and reports the accuracy cost tensor by tensor, block by block, and end to end. |

| `eval <detector.onnx> <f32.fkb> <images> <pairs.txt> [--int8 ...] [--pick ...]` | 7 | Runs the pipeline over a pair protocol such as LFW and reports the ten-fold accuracy and the true-accept rates, for the `f32` and the integer recognizer side by side. |
| `bank <detector.onnx> <f32.fkb> <images> <out.fkb> [--int8 ...] [--count N]` | 7 | Embeds one photo each of N people into the impostor bank the firmware carries. |
| `calibrate <detector.onnx> <f32.fkb> <images> <bank.fkb> [--int8 ...]` | 7 | Plays the application many times over a folder of labelled photos and reports where to put the two thresholds. |

The last three need a folder with one subfolder of photos per person:
`data/lfw/lfw_funneled` is one.

## Photos

`crops`, `quantize`, `eval`, `bank` and `calibrate` work on photos of
faces. **The photos are not part of the repository** (290 MB);
[`data/README.md`](data/README.md) says step by step how to download or
make them. A command that does not find its photos stops and names that
page. The other commands, the tests and the firmware do not need them.

## Files

| Module | Contents |
| --- | --- |
| `src/blob.rs` | writes FKB1 files; the reader is `hack_and_hike_vision::blob` |
| `src/names.rs` | the firmware names of tensors and nodes, derived from the PyTorch module paths |
| `src/onnx.rs` | reading the ONNX protobuf; running a model in tract with chosen intermediate outputs |
| `src/synthetic.rs` | the drawn face and the noise image of the golden files |
| `src/tensors.rs` | an FKB1 file in memory, usable as the networks' `Weights`, with the weights grouped for the vector kernels as the board does |
| `src/recognizer.rs` | the integer recognizer with its buffers, as the board runs it |
| `src/data.rs` | the photo folders, and the message when the photos are missing |
| `src/crops.rs` | faces from photos: aligned crops and board-like frames |
| `src/quantize.rs` | calibration (ranges, histograms, least-error clips), weight quantization, LayerNorm folding, the accuracy report |
| `src/embed.rs` | photos to embeddings with the firmware's own pipeline, on many threads |
| `src/eval.rs` | the pair protocol: ten-fold accuracy and true-accept rates |
| `src/bank.rs` | the impostor bank |
| `src/calibrate.rs` | the two thresholds of the decision |

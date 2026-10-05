# facekit

The developer-machine side of the face ID application. It runs on your
computer, never on the board, and it is plain Rust: no Python anywhere.

It turns the downloaded neural-network models into data the firmware can
use, and it measures how well the whole pipeline works, using the same
`crates/vision` code that runs on the board. The detector (YuNet) comes
as an ONNX file in `f32` and is exported and quantized here; the
recognizer (Espressif's MFN_S8_V1) comes already quantized as an
`.espdl` file and is imported.

## Build and run

facekit is a workspace of its own, built with the stable toolchain from
`rust-toolchain.toml` in this folder. Always run Cargo from this folder:
the repository root's Cargo configuration builds for the ESP32-S3, and
this folder's `.cargo/config.toml` overrides that with the host target.

```bash
cd tools/facekit
cargo run --release -- inspect --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx
cargo run --release -- import-espdl models/human_face_feat_mfn_s8_v1.espdl ../../assets/models/mfn_s8_v1.fkb
```

The model files are in `models/`; [`models/README.md`](models/README.md)
says where they come from and under which licences.

## Commands

| Command | Step | What it does |
| --- | --- | --- |
| `inspect <model.onnx> [--strict] [--input-shape ...]` | 1 | Lists operators, weights and shapes; checks every operator against the list the firmware implements; loads the model through tract. The results for YuNet are in [`docs/face_id/`](../../docs/face_id/). |
| `export <model.onnx> <out.fkb> --input-shape ...` | 3 | Writes every weight as an FKB1 file in the layouts the firmware's kernels want (`src/export.rs` has the table), renames unnamed weights after their module path, folds run-time constants, and writes a `.txt` listing next to the file. |
| `golden yunet <model.onnx> <out.fkb> [--image photo.jpg] [--source face\|noise]` | 3 | Runs the model in tract on a photo or a synthetic image and writes the input, every block boundary and the outputs as an FKB1 file, for the firmware tests. |
| `import-espdl <model.espdl> <out.fkb>` | 3 | Reads Espressif's MFN_S8_V1, checks that its graph is the one `nn::mfn` implements and that it keeps every assumption of the 8-bit kernels, merges the layers ESP-DL splits into halves, and writes the FKB1 file the firmware runs (`src/espdl.rs` has the table). |
| `golden-espdl <model.espdl> <crop.jpg> <out.fkb>` | 3 | Runs MFN_S8_V1's graph in an interpreter that shares no code with the firmware and writes the quantized input and the embedding, for the firmware tests. |
| `crops <yunet.onnx> <photos> <out>` | 6 | Finds the faces in a folder of photos (YuNet in tract at the photo's size, then the firmware's own decoding and alignment) and writes `out/crops/` (112x112 aligned crops) and `out/frames/` (320x240 frames with the face filling the height). |
| `quantize yunet <f32.fkb> <samples> <out.fkb> [--limit N]` | 6 | Calibrates the detector on the samples (320x240 frames), quantizes the weights, writes the integer file, and reports the accuracy cost tensor by tensor, block by block, and end to end. |
| `eval <detector.onnx> <recognizer.fkb> <images> <pairs.txt> [--reference model.espdl] [--pick ...]` | 7 | Runs the pipeline over a pair protocol such as LFW and reports the ten-fold accuracy and the true-accept rates of the recognizer as the board runs it; with `--reference`, also of the `.espdl` interpreter, and how many embeddings the two share bit for bit. |
| `bank <detector.onnx> <recognizer.fkb> <images> <out.fkb> [--count N]` | 7 | Embeds one photo each of N people into the impostor bank the firmware carries. |
| `calibrate <detector.onnx> <recognizer.fkb> <images> <bank.fkb> [--templates N] [--people N] [--impostors N]` | 7 | Enrolls `--people` people (120) with `--templates` photos each (5) and lets `--impostors` strangers (400) try to pass as each of them. Reports where to put the two thresholds, the scores of one, two and three frames averaged with the steps above the limit from which a decision is sure, and a play of the application in visits of five frames: how soon a person is named, and whether a stranger is. |

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
| `src/main.rs` | the command line: one subcommand per step |
| `src/inspect.rs` | `inspect`: operators, weights and shapes of an ONNX model, checked against what the firmware implements |
| `src/export.rs` | `export`: a model's weights as one FKB1 file, in the layouts of the firmware's kernels |
| `src/golden.rs` | `golden`: reference inputs, block outputs and outputs from tract, for the tests |
| `src/blob.rs` | writes FKB1 files; the reader is `hack_and_hike_vision::blob` |
| `src/names.rs` | the firmware names of tensors and nodes, derived from the PyTorch module paths |
| `src/onnx.rs` | reading the ONNX protobuf; running a model in tract with chosen intermediate outputs |
| `src/synthetic.rs` | the drawn face and the noise image of the golden files |
| `src/tensors.rs` | an FKB1 file in memory, usable as the networks' `Weights`, with the weights grouped for the vector kernels as the board does |
| `src/espdl.rs` | `.espdl` files: the FlatBuffer reader, the graph interpreter (the recognizer's reference), `import-espdl` and `golden-espdl` |
| `src/mfn.rs` | the recognizer as the board runs it (`nn::mfn`), with the interpreter beside it |
| `src/data.rs` | the photo folders, and the message when the photos are missing |
| `src/crops.rs` | faces from photos: aligned crops and board-like frames |
| `src/quantize.rs` | the detector's calibration (ranges, histograms, least-error clips), weight quantization, the accuracy report |
| `src/embed.rs` | photos to embeddings with the firmware's own pipeline, on many threads |
| `src/eval.rs` | the pair protocol: ten-fold accuracy and true-accept rates |
| `src/bank.rs` | the impostor bank |
| `src/calibrate.rs` | the two thresholds of the decision, and the sure steps |

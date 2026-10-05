# Test fixtures

Binary files in the FKB1 format (see `crates/vision/src/blob.rs`), written
by `tools/facekit`. The tests of this crate include them with
`include_bytes!`.

| File | Contents | Made by |
| --- | --- | --- |
| `yunet.f32.fkb`, `yunet.f32.txt` | the weight tensors of YuNet (2026may variant) in the firmware's layouts, `f32`, and one line per tensor: name, type, layout, shape | `facekit export --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx` |
| `face_320x240.jpg` | a portrait photo, cut so that the face fills the frame height, at the board's frame size | ImageMagick, from the photo below |
| `face_112x112.jpg` | the same photo, cut to a square around the face, roughly the way the alignment step will cut it | ImageMagick, from the photo below |
| `yunet.golden.fkb` | `face_320x240.jpg` scaled to 80x60 in a black 96x64 frame, every backbone and neck stage, the 12 head outputs, from tract | `facekit golden yunet --image face_320x240.jpg models/face_detection_yunet_2026may.onnx` |
| `mfn.golden.fkb`, `mfn.golden.txt` | `face_112x112.jpg` quantized as MFN_S8_V1's input, and its embedding, from facekit's interpreter of the `.espdl` graph | `facekit golden-espdl models/human_face_feat_mfn_s8_v1.espdl face_112x112.jpg` |

The firmware's own weights and the impostor bank are not here but in
`assets/models/`, because the firmware carries them; the tests include
them from there. The commands that make them are below.

## The photo

The two `face_*.jpg` files are cut from the official NASA portrait of
astronaut Jessica Watkins, a work of the United States government and in
the public domain:
<https://commons.wikimedia.org/wiki/File:Jessica_Watkins_Astronaut_portrait_(cropped).jpg>.
From the 1280x1874 rendering on Wikimedia Commons:

```bash
convert watkins.jpg -crop 1067x800+107+300 +repage -resize 320x240! -strip -quality 92 face_320x240.jpg
convert watkins.jpg -crop 743x743+269+258 +repage -resize 112x112! -strip -quality 92 face_112x112.jpg
```

YuNet finds the face in `yunet.golden.fkb` with score 0.93 (the square
root of `cls * obj`) at the stride-8 anchor in row 4, column 4, where the
face centre is. The tests check that.

## Making them again

From `tools/facekit`, after downloading the models as
`tools/facekit/models/README.md` says:

```bash
F=../../crates/vision/tests/fixtures
A=../../assets/models
cargo run --release -- export --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx $F/yunet.f32.fkb
cargo run --release -- golden yunet --image $F/face_320x240.jpg models/face_detection_yunet_2026may.onnx $F/yunet.golden.fkb
cargo run --release -- import-espdl models/human_face_feat_mfn_s8_v1.espdl $A/mfn_s8_v1.fkb
cargo run --release -- golden-espdl models/human_face_feat_mfn_s8_v1.espdl $F/face_112x112.jpg $F/mfn.golden.fkb
```

`import-espdl` checks that the graph is the one `nn::mfn` implements and
prints every layer with its shifts. `golden-espdl` runs the graph in an
interpreter that shares no code with the firmware; `tests/mfn.rs`
requires the firmware's embedding to be the same bit for bit.

The detector's integer file needs the calibration set: the photos of
faces in `tools/facekit/data/portraits/` (public-domain NASA portraits
from Wikimedia Commons). The photos are not part of the repository;
`tools/facekit/data/README.md` says how to download them. The first
command below makes `data/calib` from them:

```bash
cargo run --release -- crops models/face_detection_yunet_2026may.onnx data/portraits data/calib
cargo run --release -- quantize yunet $F/yunet.f32.fkb data/calib/frames $A/yunet.int8.fkb
```

It prints, per tensor, the loss of its mapping, and, per block, the
signal-to-noise ratio of the integer network against the `f32` one, then
the box and landmark differences. Different calibration photos give
slightly different mappings; the tests only check what the application
cares about. The recognizer needs no calibration: Espressif published it
quantized.

The commands are deterministic: running them again gives byte-identical
files. Only regenerate them when the export format or the models change,
because every version of a weights file stays in the git history.

Without `--image`, `facekit golden` uses a drawn face instead; that is
enough to check arithmetic, but the detector finds nothing in it.

The impostor bank needs Labeled Faces in the Wild in
`tools/facekit/data/lfw/`, which is not part of the repository either
(`tools/facekit/data/README.md` says how to download it):

```bash
cargo run --release -- bank models/face_detection_yunet_2026may.onnx \
    $A/mfn_s8_v1.fkb data/lfw/lfw_funneled $A/impostors.fkb --count 200
```

It holds what the recognizer produces on the board.

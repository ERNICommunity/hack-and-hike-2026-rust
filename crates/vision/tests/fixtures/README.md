# Test fixtures

Binary files in the FKB1 format (see `crates/vision/src/blob.rs`), written
by `tools/facekit`. The tests of this crate include them with
`include_bytes!`.

| File | Contents | Made by |
| --- | --- | --- |
| `edgeface_xxs.f32.fkb` | the 161 weight tensors of EdgeFace-XXS in the firmware's layouts, `f32` | `facekit export --input-shape 1,3,112,112 models/edgeface_xxs.onnx` |
| `edgeface_xxs.f32.txt` | one line per tensor of the file above: name, type, layout, shape | same command |
| `yunet.f32.fkb`, `yunet.f32.txt` | the weight tensors of YuNet (2026may variant) | `facekit export --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx` |
| `face_320x240.jpg` | a portrait photo, cut so that the face fills the frame height, at the board's frame size | ImageMagick, from the photo below |
| `face_112x112.jpg` | the same photo, cut to a square around the face, roughly the way the alignment step will cut it | ImageMagick, from the photo below |
| `edgeface_xxs.golden.fkb` | `face_112x112.jpg`, the model input, the output of every block and the embedding, from tract | `facekit golden edgeface --image face_112x112.jpg models/edgeface_xxs.onnx` |
| `yunet.golden.fkb` | `face_320x240.jpg` scaled to 80x60 in a black 96x64 frame, every backbone and neck stage, the 12 head outputs, from tract | `facekit golden yunet --image face_320x240.jpg models/face_detection_yunet_2026may.onnx` |

The integer weights and the impostor bank are not here but in
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

To make them again, from `tools/facekit` (after downloading the models as
`tools/facekit/models/README.md` says):

```bash
F=../../crates/vision/tests/fixtures
cargo run --release -- export --input-shape 1,3,112,112 models/edgeface_xxs.onnx $F/edgeface_xxs.f32.fkb
cargo run --release -- export --input-shape 1,3,64,96 models/face_detection_yunet_2026may.onnx $F/yunet.f32.fkb
cargo run --release -- golden edgeface --image $F/face_112x112.jpg models/edgeface_xxs.onnx $F/edgeface_xxs.golden.fkb
cargo run --release -- golden yunet --image $F/face_320x240.jpg models/face_detection_yunet_2026may.onnx $F/yunet.golden.fkb
```

The integer files need the calibration set: the photos of faces in
`tools/facekit/data/portraits/` (public-domain NASA portraits from
Wikimedia Commons). The photos are not part of the repository;
`tools/facekit/data/README.md` says how to download them. The first
command below makes `data/calib` from them:

```bash
cargo run --release -- crops models/face_detection_yunet_2026may.onnx data/portraits data/calib
A=../../assets/models
cargo run --release -- quantize edgeface $F/edgeface_xxs.f32.fkb data/calib/crops $A/edgeface_xxs.int8.fkb
cargo run --release -- quantize yunet $F/yunet.f32.fkb data/calib/frames $A/yunet.int8.fkb
```

Both print, per tensor, the loss of its mapping, and, per block, the
signal-to-noise ratio of the integer network against the `f32` one, then
the number the application cares about: embedding cosine (edgeface) or
box and landmark differences (yunet). Different calibration photos give
slightly different mappings; the tests only check what the application
cares about.

The commands are deterministic: running them again gives byte-identical
files. Only regenerate them when the export format or the models change,
because every version of the 5 MB weights file stays in the git history.

Without `--image`, `facekit golden` uses a drawn face instead; that is
enough to check arithmetic, but the detector finds nothing in it.

The impostor bank needs Labeled Faces in the Wild in
`tools/facekit/data/lfw/`, which is not part of the repository either
(`tools/facekit/data/README.md` says how to download it):

```bash
cargo run --release -- bank models/face_detection_yunet_2026may.onnx \
    $F/edgeface_xxs.f32.fkb data/lfw/lfw_funneled $A/impostors.fkb \
    --int8 $A/edgeface_xxs.int8.fkb --count 200
```

It holds what the integer recognizer produces, because that is what the
board runs.

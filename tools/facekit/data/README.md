# The photos facekit works with

Three facekit commands measure or tune the pipeline on photos of real
faces, and two more prepare those photos. The photos are **not part of
the repository**: they are 290 MB, and anyone can download them again.
This page says how. Everything else (the ONNX models, the pair protocol,
the list of portraits) is committed.

You only need the photos to regenerate the integer models or the impostor
bank, or to measure accuracy. Building and flashing the firmware does not
need them: the generated files are committed in `assets/models/`.

| Folder | What | Size | Needed by |
| --- | --- | --- | --- |
| `portraits/` | 28 public-domain NASA portraits and crew photos | 14 MB | `facekit crops` |
| `calib/crops/`, `calib/frames/` | the faces of those photos, as the two networks see them: the calibration set | 9 MB | `facekit quantize` |
| `lfw/lfw_funneled/` | Labeled Faces in the Wild: 13,233 photos of 5,749 people | 266 MB | `facekit eval`, `facekit bank`, `facekit calibrate` |

When one of these folders is missing or empty, the command stops with a
message that names this page.

All commands below are run from `tools/facekit`, and need `curl` and
`tar`, which the development container has.

```bash
cd tools/facekit
```

## 1. The portraits (`data/portraits/`)

[`portraits.tsv`](portraits.tsv) lists the 28 photos: the file name, the
download address on Wikimedia Commons, and the page that describes the
photo and its licence (all are works of NASA and in the public domain).
Download them with:

```bash
mkdir -p data/portraits
grep -v '^#' data/portraits.tsv | while IFS=$'\t' read -r file url page; do
  curl -sSL --fail -A "facekit" -o "data/portraits/$file" "$url" || echo "failed: $file"
  sleep 1
done
ls data/portraits | wc -l    # 28
```

The `sleep` keeps Wikimedia from refusing the requests. The addresses ask
for the photos at a fixed width, so the files are byte for byte the ones
the committed models were calibrated on.

You can use photos of your own instead, or add some: any `jpg`, `jpeg` or
`png` directly in the folder is used. Choose photos with faces at least
100 pixels high, of different people, in different light. The mappings of
the integer models then differ slightly from the committed ones.

## 2. The calibration set (`data/calib/`)

Made from the portraits by facekit, not downloaded:

```bash
cargo run --release -- crops models/face_detection_yunet_2026may.onnx data/portraits data/calib
ls data/calib/crops | wc -l     # 64
ls data/calib/frames | wc -l    # 64
```

This finds every face in every portrait and writes it twice:
`data/calib/crops/` holds the 112x112 aligned faces the recognizer is
calibrated on, `data/calib/frames/` the 320x240 frames the detector is
calibrated on. Then the integer models can be made:

```bash
F=../../crates/vision/tests/fixtures
A=../../assets/models
cargo run --release -- quantize edgeface $F/edgeface_xxs.f32.fkb data/calib/crops $A/edgeface_xxs.int8.fkb
cargo run --release -- quantize yunet $F/yunet.f32.fkb data/calib/frames $A/yunet.int8.fkb
```

`crates/vision/tests/fixtures/README.md` has the whole sequence.

## 3. Labeled Faces in the Wild (`data/lfw/lfw_funneled/`)

```bash
cd data/lfw
curl -L -o lfw-funneled.tgz https://ndownloader.figshare.com/files/5976015
tar xzf lfw-funneled.tgz && rm lfw-funneled.tgz
find lfw_funneled -mindepth 1 -maxdepth 1 -type d | wc -l    # 5749 people
cd ../..
```

The download is 233 MB. The pair protocol, `data/lfw/pairs.txt`, is
committed; [`lfw/README.md`](lfw/README.md) describes it and says where
the files come from. Then:

```bash
F=../../crates/vision/tests/fixtures
A=../../assets/models
M=models/face_detection_yunet_2026may.onnx
cargo run --release -- eval $M $F/edgeface_xxs.f32.fkb data/lfw/lfw_funneled data/lfw/pairs.txt --int8 $A/edgeface_xxs.int8.fkb
cargo run --release -- bank $M $F/edgeface_xxs.f32.fkb data/lfw/lfw_funneled $A/impostors.fkb --int8 $A/edgeface_xxs.int8.fkb --count 200
cargo run --release -- calibrate $M $F/edgeface_xxs.f32.fkb data/lfw/lfw_funneled $A/impostors.fkb --int8 $A/edgeface_xxs.int8.fkb
```

Add `--limit 600` to `eval` for a quick check on the first 600 pairs.

Any folder with one subfolder of `jpg` photos per person works in place
of `lfw_funneled` for `bank` and `calibrate`; `eval` also needs a pair
file that names those photos.

## Checking what git sees

```bash
git status --short --ignored tools/facekit/data
```

lists the three photo folders with `!!` (ignored). They are named in
`tools/facekit/.gitignore`; do not add them with `git add -f`.

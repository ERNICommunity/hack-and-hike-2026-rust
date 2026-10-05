# Face ID

The face recognition application (`src/bin/face_id.rs`): what it is made
of, what was decided on the way, and what was measured. The work went in
numbered steps, from the models on the computer to the application on the
board; the sections below keep that order.

Steps 1 to 10 were done with EdgeFace-XXS as the recognizer. Since
`faceid-16` the recognizer is Espressif's MFN_S8_V1, for its licence:
see [The recognizer since `faceid-16`](#the-recognizer-since-faceid-16-mfn_s8_v1).
The findings about EdgeFace-XXS below are kept as the record of the
work; its code and weights are in the git history.

| Part | Where |
| --- | --- |
| The application | `src/bin/face_id.rs` |
| Image processing, the two networks, the gallery and the decision | `crates/vision/` (runs on the board and on the computer) |
| The developer tool: export, calibration, evaluation | `tools/facekit/` |
| The models the firmware carries | `assets/models/` |
| What the board measured, and what is left to gain | [`performance.md`](performance.md) |

| File | Contents |
| --- | --- |
| [`performance.md`](performance.md) | the speed work on the board: the cost model, the builds, what went wrong, room for improvement |
| [`kernels.md`](kernels.md) | every ONNX operator of the two models and how the firmware handles it (Step 1 acceptance) |
| [`inventory_yunet_2023mar.md`](inventory_yunet_2023mar.md) | `facekit inspect` of the YuNet detector, fixed 640x640 variant |
| [`inventory_yunet_2026may.md`](inventory_yunet_2026may.md) | `facekit inspect` of the YuNet detector, dynamic variant, at 64x96 |

## Models

| Role | Model | Parameters | Input | Output | Licence |
| --- | --- | --- | --- | --- | --- |
| Detector + 5 landmarks | YuNet (OpenCV Zoo, 2026may dynamic variant) | 53,121 | `[1,3,H,W]` BGR, values 0..255, H and W multiples of 32 | per stride 8/16/32: `cls`, `obj`, `bbox[4]`, `kps[10]` per anchor | MIT |
| Face embedding, since `faceid-16` | MFN_S8_V1 (Espressif ESP-DL, a MobileFaceNet), all `i8` | 1,186,321 | `[1,112,112,3]` RGB, `(x - 127.5) / 127.5` in units of 2^-6 | `[512]` | MIT |
| Face embedding, up to `faceid-15` | EdgeFace-XXS (Idiap; ONNX export by yakhyo) | 1,244,744 | `[1,3,112,112]` RGB, `(x/255 - 0.5) / 0.5` | `[1,512]` | CC BY-NC-SA 4.0 |

The input conventions above are the published ones for the model families;
Step 3 (`facekit golden`) verified them against a real face before anything
depended on them, and `facekit golden-espdl` does so for MFN_S8_V1.

## Findings from Step 1

- **YuNet needs input sides that are multiples of 32.** Its neck upsamples
  the stride-32 map by 2 and adds it to the stride-16 map, and again to the
  stride-8 map. With 80x60 the maps are 2x5 and 3x10 rows and the addition
  fails. The board therefore downscales the 320x240 frame by 4 to 80x60 and
  places it in the top-left corner of a black 96x64 detector input. Anchors
  over the black border never fire. At 96x64 the model has 96 + 24 + 6 = 126
  anchors in total.
- **EdgeFace computes its positional encoding at run time** from the input
  size (`Sin`, `Cos`, `CumSum`, `Not` nodes in the inventory). For a fixed
  112x112 input it is a constant. `facekit export` will evaluate it once and
  store the result, so the firmware never runs those operators.
- **tract needs a concrete input shape** to optimize either model
  (`--input-shape 1,3,112,112` and `--input-shape 1,3,64,96`). With the
  symbolic `batch_size` of the EdgeFace export, its optimizer fails in the
  positional-encoding subgraph. Every later facekit command fixes the shape.
- YuNet's `Resize` is `mode=nearest`, scale 2: a plain pixel duplication.
- EdgeFace's GELU is exported as `x * 0.5 * (1 + Erf(x / sqrt(2)))`: the
  exact form, not the tanh approximation.

## Findings from Step 3

- **The FKB1 file format** (`crates/vision/src/blob.rs`) carries named
  tensors with a layout string. Weights are stored in the layouts the
  channels-last kernels want: `OHWI` for full convolutions, `HWC` for
  depthwise ones, `OI` for linear layers. The `.txt` listing next to each
  weights file is the reference for Step 4: every tensor name the forward
  pass reads comes from there.
- **Only stage 1 of EdgeFace has a positional encoding** (EdgeNeXt enables
  it per stage). `export` folds it into `stages.1.blocks.1.pos_embd.constant`,
  a `[14][14][48]` tensor. The other two attention blocks add nothing.
- **EdgeFace's `embedding` output is not normalized** (norm of the synthetic
  face's embedding is far from 1). The firmware L2-normalizes before it
  compares, as EdgeFace's own code does.
- **Attention has 4 heads in every stage** (`xca.temperature` is `[4]`):
  channels per head are 12, 22 and 42.
- **Block boundaries in the golden files** are the last `Add` of each
  block. Inside the attention blocks the golden files also hold the
  intermediate `Add`s: the token + positional-encoding sum (`Add_1` of
  stage 1, shape `[1, 196, 48]` = tokens x channels), the split-convolution
  chain, and the residual after attention.
- **Fixture size**: 5.0 MB of `f32` weights plus 1.5 MB of golden vectors
  are committed under `crates/vision/tests/fixtures/`. Regenerate rarely.
- **The golden files come from a real face**: a public-domain NASA portrait
  (see the fixtures README), cut so the face fills the frame height as the
  application demands. YuNet scores it 0.93 at the stride-8 anchor under
  the face centre, and the other two strides stay quiet (0.04 and 0.01):
  a face that fills a 60-pixel-high frame is a stride-8 detection. The
  tests assert the score and position, so Step 5's decoder has a known
  answer to hit.

## Findings from Step 4

- **The `f32` reference of EdgeFace-XXS matches tract at every block.**
  `crates/vision/src/nn/edgeface.rs` follows the PyTorch source block by
  block; `tests/edgeface.rs` compares all 26 block outputs and the
  embedding with the golden file at `1e-4 + 1e-4 * max|golden|`. The
  errors are 1e-7 to 1e-6 for most blocks and 1e-5 at the three residual
  sums whose values reach 13: pure `f32` summation-order noise. The exact
  GELU (`erf`) and the `F.normalize` epsilon of the attention are matched
  as written; nothing needed tuning.
- **Channels-last needs no transposes anywhere in EdgeFace.** The
  attention block's "pixels as tokens" view, `[token][channel]`, is the
  same memory as `[row][column][channel]`. The 45 `Transpose` nodes of
  the ONNX graph all disappear.
- **The split-convolution chunks are `ceil(C / (n + 1))` wide with a
  shorter last chunk** (PyTorch `chunk`): 24+24 in stage 1, 30+30+28 in
  stage 2, 42x4 in stage 3. The exported split-conv weights confirm it
  (`convs.0.weight` is `[3, 3, 30]` in stage 2).
- **Scratch memory of the `f32` reference**: `SCRATCH_LEN` = 4 x 18,816
  + 75,264 + 7,056 values = 630 KB. That is the host reference; the
  integer version of Step 6 shrinks it, and the widest tensor (28x28x96
  in the stage-0 MLP) is the one to fuse per pixel so it never exists.
- **The `f32` reference of YuNet matches tract at every node.**
  `crates/vision/src/nn/yunet.rs`; `tests/yunet.rs` compares the 21
  Relu/MaxPool/Add outputs and the 12 head outputs, all at ~1e-6. The
  `Resize` + `Add` pairs of the neck are one fused `upsample_2x_add`, and
  the neck adds into the backbone's own map buffers, so the stride-8 and
  stride-16 maps never move. The reference finds the fixture face at the
  stride-8 anchor (row 4, column 4) with score 0.932, as tract does.
- **Scratch memory of the YuNet reference**: 60,192 values (240 KB as
  `f32`, 60 KB as `i8`), dominated by the two 32x48x16 work buffers of the
  stem. Small enough for internal RAM once it is `i8`.
- **Kernel set**: the whole of both networks runs on nine `f32` kernels
  (`conv2d`, `depthwise`, `linear`, `layer_norm`, `gelu`, `sigmoid`,
  `softmax_rows`, `max_pool_2x2`, `upsample_2x_add`) plus three trivial
  ones (`global_average`, `add`, `add_scaled`) and one composite
  (cross-covariance attention, written out in `edgeface.rs`). Those are
  the functions Step 6 quantizes and the board work vectorizes.

## Findings from Step 5

- **Decoding follows OpenCV's `FaceDetectorYN`** (`crates/vision/src/detect.rs`):
  `score = sqrt(cls * obj)`, centre `(column + dx, row + dy) * stride`,
  size `exp(dw, dh) * stride`, landmarks `(column + kx, row + ky) * stride`,
  then greedy non-maximum suppression at IoU 0.3. On the fixture the
  decoder gives one face after suppression: score 0.932, box (18.3, 1.7)
  42.6x54.4 in the 96x64 detector image, i.e. the face fills 91 percent
  of the 60-pixel content height. `tests/detect.rs` decodes the anchor by
  hand from the golden head values and gets the same numbers.
- **YuNet names its landmarks by image side, not by anatomy.** Mirroring
  the input moves landmark 0 (the eye on the image's left) onto the mirror
  of landmark 1, within 5 px. So landmark `i` maps straight onto point `i`
  of the ArcFace template, no swap. The test pins this down; a future model
  swap must pass it.
- **Alignment works from the half-size frame.** Landmarks scaled to the
  160x120 image, similarity fit, bilinear warp: the five landmarks land
  1 to 3 px from the template (the network's landmark noise, not the
  warp's). The embedding of that crop has **cosine 0.957** with the
  embedding of the hand-cut 112x112 crop of the same photo; a background
  patch scores 0.045.
- **The board path reproduces the golden detection**: RGB565 frame, box
  downscale by 4 (`downscale_to_rgb`), `detector_input` (BGR, black
  padding), forward, decode: score 0.932, centre within 1 px of the golden
  run, although the golden input was made with another scaler (mean
  difference 1.8 per value).
- **Gate limits are starting points** (`gates::Limits::DEFAULT`): face
  height 75 to 105 percent of the frame, 5 percent edge margin, roll 15
  degrees, yaw 0.25, pitch 0.35 to 0.75. The fixture face measures roll
  -0.3 degrees, yaw -0.02, pitch 0.54. The sharpness of its aligned crop
  is 351 (`laplacian_variance`), 214 after a 3x3 box blur; the default
  minimum of 100 is deliberately loose until the application measures real frames
  from the camera.

## Findings from Step 6

The integer version of both networks, and how the accuracy cost was
measured and brought down. `facekit quantize` prints every number below;
`tests/int8_edgeface.rs` and `tests/int8_yunet.rs` enforce them on the
fixture face.

- **Calibration data**: 28 public-domain NASA photos (portraits and crew
  group photos, `tools/facekit/data/portraits/`, not part of the
  repository: `tools/facekit/data/README.md`), from which
  `facekit crops` cut 64 aligned 112x112 crops and 64 320x240 frames with
  the face filling 90 percent of the height. The photos are not the
  fixture face.
- **8-bit activations are not good enough.** Per-tensor 8-bit activations
  everywhere gave a mean embedding cosine of 0.95 between the integer and
  the `f32` recognizer, worst 0.89. Not a bug: each residual block added
  noise at about -30 dB of its own output, and thirteen blocks compound
  that into 6 dB at the head.
- **The scheme that runs** (`nn::lanes`, `nn::edgeface::int8`): every
  tensor between layers is **16-bit** with one symmetric mapping, the
  weights are 8-bit with one scale per output channel, and every
  LayerNorm's scale and shift is folded into the layer after it. The
  residual stream has one mapping per stage; the MLP's hidden tensor is
  in units of 2^-10, since GELU is a table in those units. An earlier
  scheme kept the residual stream in `f32` with 8-bit per-channel tensors
  at the depthwise convolutions; it was as accurate and three times
  slower on the board ([`performance.md`](performance.md)).
- **Two details that decide the accuracy.** The requantization shifts
  the eight sums of a group of channels by one common amount before each
  is scaled by its own factor, so a channel whose scale is far from its
  neighbours' keeps fewer bits. The shift is therefore taken from the
  layer's own output range, before the block's layer scale `gamma`:
  stage 2's `gamma` spans a factor of 100 inside a group, and folding it
  first cost 20 dB. And the bias enters in two parts, whole product units
  before the shift and the rest after it.
- **Result on the 64 calibration faces**: embedding cosine integer vs
  `f32` **mean 0.9972, worst 0.9955**. Genuine pairs of the same person
  score about 0.5-0.8 and impostors below 0.3, so an integer noise of
  0.003 is far below what any threshold sees.
- **A bug the report caught**: the LayerNorm fold rule matched
  `head.norm.weight` with the generic `.norm.weight` rule first and folded
  the head into a layer that does not exist. `folded_norms` now checks
  that every fold's consumer exists.
- **YuNet is all 16-bit activations, 8-bit weights**
  (`nn::yunet::int8`). What is left of its noise is the 8-bit weights
  (42-48 dB each). On the fixture the integer detector's box and
  landmarks are within 0.2 px of the `f32` ones (detector pixels, a
  quarter of frame pixels); on the 64 calibration frames it finds the same
  face on every frame, within 2.2 px centre and 3.2 px landmark at worst.
  The same holds since its backbone and neck run on the lane kernels
  (`faceid-12`, see [`performance.md`](performance.md)).
- **File sizes**: `edgeface_xxs.int8.fkb` 1.37 MB, `yunet.int8.fkb` 92 KB.
- **Scratch memory**: the recognizer 472 KB and the detector 359 KB, both
  in PSRAM, plus the GELU table (15 KB) and a strip of the recognizer's
  MLP hidden tensor (36 KB) in internal RAM. The compiled recognizer
  also keeps its early layers' weights as `i16` (123 KB in PSRAM).

## Findings from Step 7

The question this step answers is the one the golden vectors cannot: not
"does the firmware compute the same numbers as the original model?" but
"are those numbers any good at telling faces apart?".

- **Labeled Faces in the Wild**, the benchmark EdgeFace published its
  accuracy on, is in `tools/facekit/data/lfw/` (that folder's README
  says where it comes from). 13,233 photos, 5,749 people, and
  a 6,000-pair protocol in ten folds. The original UMass site did not
  answer; the files come from the mirror scikit-learn uses.
- **The pipeline scores 99.35 % +- 0.42 on LFW** (`facekit eval`),
  against EdgeFace-XXS's published 99.57 %. The remaining 0.22 points are
  most likely the landmark convention: EdgeFace's own alignment uses
  MTCNN's five points, ours uses YuNet's, and a detector's landmarks sit
  in systematically different places. The alignment template itself is
  confirmed correct — EdgeFace aligns to the same ArcFace 112x112 points
  that `warp::ARCFACE_TEMPLATE_112` holds.
- **The integer recognizer scores 99.42 % +- 0.37**, 0.07 points above
  the `f32` one, which is noise. Quantization costs nothing measurable at
  the level the application works at. Over the 7,701 photos the two
  embeddings agree at cosine 0.9970 on average, 0.909 at worst.
- **A bug in the measurement, not the firmware.** The first run gave
  98.05 %. The cause was `eval` picking the face the detector scored
  highest, and LFW photos often have bystanders, so some pairs compared
  the wrong person. Picking the face nearest the photo's centre, which is
  the subject by LFW's construction, lifted it to 99.35 %. The firmware
  is unaffected: its framing gate already requires one face filling the
  frame. `--pick score|centre|largest` keeps the choice visible.
- **The impostor bank** (`facekit bank`) holds 200 strangers from LFW,
  one photo each, as `i8` with a single scale: 102 KB of flash instead of
  409 KB, and the rounding moves a cosine by at most 0.0001. No two
  members score above 0.374 against each other, so no person is in it
  twice.
- **The thresholds** (`facekit calibrate`) come from playing the
  application 120 times over: enroll one person from five photos, then
  let that person and 191 strangers try to be recognized, 24,271 attempts
  in all, scored by the firmware's own `Gallery::match_probe` (the sweep's
  rule is checked against it on 2,460 decisions). The chosen point is
  **accept 0.35, margin 0.0**: 98.8 percent of genuine attempts
  recognized, 0.02 percent of strangers let in (five of 22,920).
- **Averaging three frames is worth about 1.3 points**: at the threshold
  where no stranger at all gets in, one frame recognizes 98.7 percent and
  three frames 100 percent.
- **A sure face needs one frame** (`gallery::SureSteps`, measured by
  `facekit calibrate` with the firmware's own `gallery::Decider`). The
  highest score of a stranger was 0.430 on one frame (22,920 attempts),
  0.369 on two frames averaged (11,400) and 0.401 on three (7,560). A
  decision is sure from 0.55 on one frame, 0.50 on two and 0.45 on
  three: 0.20, 0.15 and 0.10 above the accept threshold, and at least
  0.05 above every stranger. Over 223 visits of five frames by an
  enrolled person, the name appeared after the first frame in 95.5
  percent of them and after the second in 98.7 percent. The rule without
  the shortcut named 97.3 percent, after five frames. Of 27,480 visits by
  strangers, both rules named the same five. Steps of 0.15, 0.10 and 0.05
  named three of those five sooner, and one stranger more.
- **The margin rule did not earn its keep on this data.** Sweeping it
  freely, including negative values that switch it off, the optimum was
  always at or below zero, and raising it to 0.10 cost two points of
  recognition while stopping no further stranger. It stays in the code at
  margin 0 (a face must still beat the closest stranger, but by nothing
  in particular) because what it defends against — every score drifting
  together when the light changes — is exactly what LFW cannot show: both
  photos of an LFW pair are already "in the wild". Step 10 is where it
  gets its real verdict, on the device.

## The recognizer since `faceid-16`: MFN_S8_V1

EdgeFace-XXS's weights are licensed CC BY-NC-SA 4.0: no commercial use,
and every copy and derivative under the same terms. Of the face
recognizers with a permissive licence, only Espressif's MFN_S8_V1 (MIT,
`esp-dl/models/human_face_recognition`) is small enough for the board:
OpenCV's SFace (Apache-2.0) is 9.9 MB even in `int8`, fal's AuraFace a
ResNet100, dlib's model a ResNet at 150x150. Espressif does not say what
MFN_S8_V1 was trained on.

- **Espressif publishes it quantized**, as an `.espdl` file: ONNX in a
  FlatBuffer, every tensor `i8` with one power-of-two scale. Espressif's
  loader is a closed library; `facekit` reads the format itself
  (`tools/facekit/src/espdl.rs`), interprets the graph as the reference,
  and imports it (`import-espdl`), checking every assumption of the
  firmware's kernels. ESP-DL splits three layers into two halves with
  scales of their own; the import merges each pair into one layer whose
  groups of sixteen channels carry their own shifts.
- **The network** (`nn::mfn`) is a MobileFaceNet: a 3x3 stem, fifteen
  blocks of a widening 1x1, a depthwise 3x3 and a narrowing 1x1 (three of
  them halve the image), PReLU after nearly every layer, and a head with
  a 7x7 depthwise layer over the whole map. 221 million products per
  face, more than twice EdgeFace-XXS's; 93 percent in 1x1 layers.
- **It runs on the vector unit's 8-bit mode** (`nn::s8`): sixteen
  `i8 x i8` products per instruction, twice the 16-bit lanes. Each block
  runs in bands of rows whose wide tensors stay in internal RAM;
  `performance.md` has the measurements behind that.
- **The firmware computes what the interpreter computes, bit for bit**,
  on the fixture face (`tests/mfn.rs`) and on all 7,701 LFW photos of the
  pair protocol (`facekit eval --reference`).
- **LFW: 99.27 % +- 0.54** with the firmware's detector and alignment
  (`--pick centre`), against 99.42 % for EdgeFace-XXS; at false-accept
  rates of 1e-3 and 1e-4 it accepts 98.23 % and 97.60 % (EdgeFace-XXS
  98.33 % and 97.50 %). Swapping the input to BGR costs 0.2 points, so
  RGB is the order it was trained on.
- **The decision moved with it.** MFN_S8_V1 scores strangers a little
  higher (0.027 on average against 0.006), so at the old accept
  threshold of 0.35 it let in 31 of 22,920 strangers. `facekit
  calibrate` chose **accept 0.40** (five of 22,920, as before, with 99.0
  percent of genuine attempts recognized against 98.8) and a sure limit
  of **0.50 on any number of frames**, 0.05 above the highest stranger:
  a name after the first frame in 98.7 percent of the visits (95.5
  before) and after the second in all, with no stranger named that the
  rule without the shortcut did not name. The impostor bank was made
  again from the same 200 strangers.
- **On the board** (`faceid-16`): 432 ms per face alone, against 383 for
  EdgeFace-XXS; about 910 ms per recognizing cycle in the application,
  against 860. The enrollments' format version went to 2: EdgeFace's
  enrollments cannot be compared with MFN_S8_V1's embeddings.

## Step 8 onward: on the board

Both networks compute on the board exactly what they compute on the
computer, bit for bit. Alone on CPU0 the recognizer takes 0.43 s per
face and the detector 0.10 s per frame (build `faceid-16`, MFN_S8_V1;
EdgeFace-XXS took 0.39 s in `faceid-13`); in the application a cycle
without a face takes 0.17 s and one that recognizes 0.91 s. The first run took 8.8 s and 1.6 s for the two networks; how
that became the present numbers, what the board taught on the way, and
what is left to gain is in [`performance.md`](performance.md).

`crates/vision/tests/fingerprints.rs` pins the numbers of the whole path
on the fixture photo, from the scaled-down frame to the embedding. A
change that is meant to keep the numbers must leave it alone. The same
file pins the outputs of both networks on made-up inputs (`nn::check`),
and the application checks those on the board when it starts: its log
says whether the board computes what the computer computes, and how long
each network takes alone.

## The application (`src/bin/face_id.rs`)

The camera's buffer overflows within milliseconds without a pump, and
the networks hold CPU0 for hundreds of milliseconds. So the camera and
the screen belong to a task on an interrupt executor of CPU0 (on the
board's spare software interrupt, `FROM_CPU_INTR2`). It interrupts the
networks every 2 ms to empty the camera's buffer, and draws the live
preview at up to 10 frames per second with the box and landmarks of the
newest detection. The main task's cycle asks it for the newest frame and
gets the frame's buffer in exchange for its own (`Frame::take`), without
a copy. It works on that frame: detector on the 4x scaled-down frame,
gates (framing, pose, sharpness), then, when the face passes the gates
and there is a reason, the recognizer on the face cut out of the 2x
scaled-down frame. It hands its panel canvas to the same task to show.

The stream task never waits while it has CPU0: it is an interrupt
handler, and while it runs, the main task and the timer interrupt of
both cores stand still. After an overflow of the camera's buffer it
starts the capture again without waiting for the sensor
(`Camera::service`), and the preview and the panel sleep while their
pixels are on the bus (`Surface::render_from_async`,
`Canvas::show_async`). And it takes as little of CPU0 as it can:

- The camera copies a frame out of its buffer only when the task asked
  for one (`Camera::capture_on_demand`): when the frame before was drawn
  or handed over. That is about ten frames per second, not every frame
  the sensor sends.
- The copy of the screen for the live feed is made only while a computer
  watches the feed (`logging::mirror_only_when_watched`), and then from
  every fourth preview (`Surface::without_mirror`): the feed shows two to
  three camera frames per second anyway.

The log's `cycle:` line reports the whole cycle and its steps in
milliseconds, and the preview's frame rate:

```text
cycle: <all> ms: capture <ms>, scale <ms>, detect <ms>, align <ms>, embed <ms>, decide <ms>; stream <ms> ms (copies of <frames> frames <ms> ms), preview <rate> fps, dropped <frames>, longest pump gap <ms> ms; <the face>, <the hint>
```

`scale` is the scaling down of the frame for the detector, `align` the
cutting out of the face for the recognizer, `decide` the comparison with
the gallery and the bank. A step that did not run is 0. `stream` is the
time CPU0 spent in the stream task during the cycle, and `copies` the
part of it that went into copying camera frames into PSRAM. `dropped`
counts the camera frames lost, and `longest pump gap` is the longest
time between two times the camera's ring buffer was emptied: it
overflows at about 5 ms.
A cycle that cuts a face out also logs its steps:

```text
align: <columns>x<rows> of 160x120 scaled in <us> us, warp <us> us, sharpness <us> us, input <us> us
```

Enrollment records six embeddings while the panel asks for small turns
of the head; people are `person 1` to `person 4`. Recognition goes
through `gallery::Decider` with the Step 7 thresholds (accept 0.35,
adjustable with **-** and **+**; margin 0):

- A person whose score is sure is named at once: from 0.55 on the first
  embedding, from 0.50 on the average of two, from 0.45 on the average
  of three. Most known faces are named after one cycle.
- Every other decision waits for the average of three embeddings. The
  first one is shown at once when it says "unknown". Every other change
  of the banner waits until three decisions in a row agree.

After a second without a usable face the banner returns to scanning.
Every hint, decision, score and timing goes to the log as well. The
panel draws only the lines that changed, and the timings at most once
per second.

At start the app copies both weights files into PSRAM, one tensor at a
time with a pause after each (so the tasks on CPU1, which run from the
same flash, keep their share of it), with the linear and convolution
weights grouped by eight output channels as the vector kernels read
them (`nn::pack`). Then it compiles both networks
(`edgeface::int8::Model::compile`, `yunet::int8::Model::compile`): it
looks every tensor up by its name and makes every plan, once. A cycle
only computes. The log says how long both took. Last, before the camera
starts, it runs the self-test (`nn::check`); the panel's terminal says
`self-test ok`, or that it failed.

### Enrollments in flash

The gallery lives in 128 KB of the 4 MB flash chip from offset
`0x3D0000`, above the application image and below the storage
capability's record (the last 64 KB), so flashing a new firmware keeps
it: `cargo dist` writes an image that ends with the application
(`--skip-padding`). Up to `faceid-13` the image was padded to the whole
flash, and every flash through autoflash erased the gallery. Up to
`faceid-14` the gallery began at `0x3E0000` and shared its last 64 KB
with the storage record; a board enrolled before `faceid-15` starts
empty once. Sector 0 is a header (magic, version, the number of people,
each slot's name and template count); sectors 1 onward hold one 24 KB
block per slot, twelve embeddings of 512 `f32` values. The app loads it
at boot and writes it when an enrollment completes (six samples) and
when the gallery is emptied; an enrollment interrupted before its sixth
sample is not stored.

Writing goes through `esp-storage`, which parks CPU1 while a sector is
erased or written, because the flash cache both cores execute from is
switched off meanwhile. Each sector write holds the firmware's critical
section from before the park: if CPU1 were parked while it held that
lock, the next interrupt handler on CPU0 that needed it would wait
forever (the first build with the store hung that way). A save takes
about a second, during which the touch and the preview pause and the
audio tasks log DMA restarts.

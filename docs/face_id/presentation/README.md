# Face Identification presentation

A 21-slide deck about the `face_id` application (“Face Identification on a
microcontroller”), in the layout of the ERNI
Brandbook v.04 (PowerPoint grid 1920 x 1080, Source Sans, dark blue
`#033778`, light blue `#00AADB`, dark grey `#515455`, light grey `#B1B0B1`).

It describes build `faceid-16`, whose recognizer is Espressif's MFN_S8_V1
(MIT). The speed-up story on slide 14 and the quantization tries on slide 11
were done with the first recognizer, EdgeFace-XXS, and say so; slide 15 is
the switch and why (EdgeFace-XXS's weights are CC BY-NC-SA 4.0).

| File | Contents |
| --- | --- |
| `face-id.pdf` | the deck to present or send; built by `render.sh` and committed, so it can be opened straight from the repository |
| `face-id.html` | the same deck as one web page (open it in a browser) |
| `src/style.css` | the brand tokens and the grid |
| `src/slides/*.html` | one file per slide |
| `build.js` | puts the slides together and draws the charts from the measured numbers |
| `render.sh` | builds the HTML, prints the PDF with headless Chromium, cuts PNG previews |
| `assets/` | the ERNI logo and coat of arms (vector, taken from the brand book), Source Sans 3 and Source Code Pro, the test photo, the test photo's real 512-number embedding, and a capture of the board's screen (`device-screen.png`, build `faceid-14`, before the recognizer switch, slide 2) |

| Slide | |
| --- | --- |
| 1–3 | cover, the idea, the challenge |
| 4–7 | architecture: the pipeline, laptop and board, cores and memory |
| 8–12 | neural networks: multiply-add, the embedding and the decision, 8-bit numbers, the vector unit |
| 13–18 | performance: the EdgeFace-XXS speed-up, the switch to MFN_S8_V1, the price list, the app, Rust |
| 19–21 | results, lessons, thanks and model credits |

Every number on the slides comes from `docs/face_id/README.md` and
`docs/face_id/performance.md`. The box and landmarks on slides 1 and 5
are the real detector output for the test photo (`tests/detect.rs`), and the
bar codes are the real embedding from `crates/vision/tests/fixtures/mfn.golden.fkb`
(tensor `embedding`, 512 `i8` values, scaled to unit length).

`face-id.pdf` and `face-id.html` are both built files, and both are
committed: the deck has to be readable without a toolchain. After an edit,
rebuild and commit them together with the slide sources, or they drift apart.

To rebuild after an edit (needs Node, Chromium and poppler-utils):

```bash
./render.sh out        # writes out/face-id.pdf and out/p-01.png ... p-21.png
```

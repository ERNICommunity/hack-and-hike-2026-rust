# Third-party notices

This repository is licensed under MIT OR Apache-2.0 (`LICENSE-MIT`,
`LICENSE-APACHE`). The parts below come from others, keep their own
licences, and are listed here with their notices. A firmware image built
from this repository contains the parts marked **(firmware)**, so this
file is also the notice that goes with such an image.

## Neural-network models (firmware)

| Part | Where | Licence |
| --- | --- | --- |
| MFN_S8_V1 face recognizer, Copyright (c) 2021 Espressif Systems (Shanghai) Co., Ltd., from ESP-DL (`models/human_face_recognition`) | `tools/facekit/models/human_face_feat_mfn_s8_v1.espdl`, imported into `assets/models/mfn_s8_v1.fkb` | MIT: [`assets/models/LICENSE-MFN_S8_V1`](assets/models/LICENSE-MFN_S8_V1) |
| YuNet face detector, Copyright (c) 2020 Shiqi Yu, from OpenCV Zoo (`models/face_detection_yunet`) | `tools/facekit/models/face_detection_yunet_*.onnx`, exported and quantized into `assets/models/yunet.int8.fkb` and the test fixtures | MIT: [`assets/models/LICENSE-YuNet`](assets/models/LICENSE-YuNet) |

The impostor bank (`assets/models/impostors.fkb`) holds 200 embeddings
that MFN_S8_V1 computed from photos of Labeled Faces in the Wild
(University of Massachusetts Amherst); `tools/facekit/data/lfw/pairs.txt`
is that dataset's pair protocol. The photos are not in the repository.

## Code and register tables

### esp32-camera (firmware)

The GC0308 camera register table and its reset sequence in
`src/capabilities/camera/gc0308.rs` come from Espressif's esp32-camera
(`sensors/private_include/gc0308_settings.h`, `sensors/gc0308.c`).

    Copyright 2015-2021 Espressif Systems (Shanghai) PTE LTD

    Licensed under the Apache License, Version 2.0 (the "License");
    you may not use this file except in compliance with the License.
    You may obtain a copy of the License at

        http://www.apache.org/licenses/LICENSE-2.0

    Unless required by applicable law or agreed to in writing, software
    distributed under the License is distributed on an "AS IS" BASIS,
    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
    See the License for the specific language governing permissions and
    limitations under the License.

Modified: written as a Rust array without the rows that are commented out
upstream; the 320x240 subsampling, the RGB565 format and the orientation
are written after it. The licence text is `LICENSE-APACHE`.

### M5Unified (firmware)

The initialization values of the ES7210 microphone ADC and the AW88298
amplifier (`src/capabilities/audio/codecs.rs`) and the start values of
the AW9523 I/O expander (`src/board/io_expander.rs`) follow M5Stack's
M5Unified library for the CoreS3.

    MIT License

    Copyright (c) 2021 M5Stack

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.

### Bosch Sensortec SensorAPI (firmware)

The BMI270 configuration blob (`src/capabilities/imu/bmi270/config.rs`,
from BMI270_SensorAPI v2.86.1, `bmi270_maximum_fifo.c`, Copyright (c) 2023
Bosch Sensortec GmbH) and the BMM150 compensation equations
(`crates/core/src/imu/bmm150/mod.rs`, translated from BMM150_SensorAPI
v2.0.0, `bmm150.c`, Copyright (c) 2020 Bosch Sensortec GmbH). Both files
carry the full notice; the licence is:

    Copyright (c) 2020, 2023 Bosch Sensortec GmbH. All rights reserved.

    BSD-3-Clause

    Redistribution and use in source and binary forms, with or without
    modification, are permitted provided that the following conditions are met:

    1. Redistributions of source code must retain the above copyright
       notice, this list of conditions and the following disclaimer.

    2. Redistributions in binary form must reproduce the above copyright
       notice, this list of conditions and the following disclaimer in the
       documentation and/or other materials provided with the distribution.

    3. Neither the name of the copyright holder nor the names of its
       contributors may be used to endorse or promote products derived from
       this software without specific prior written permission.

    THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
    "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
    LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS
    FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE
    COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT,
    INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
    (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
    SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
    HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT,
    STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING
    IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
    POSSIBILITY OF SUCH DAMAGE.

### esp-generate

`build.rs` is copied from the esp-rs project template esp-generate,
licensed MIT OR Apache-2.0, like this repository.

### The flashing page (`tools/autoflash/site/`)

The built page bundles esptool-js (Apache-2.0, Espressif), pako (MIT and
Zlib), atob-lite (MIT) and tslib (0BSD). Their licence texts are in
[`tools/autoflash/THIRD_PARTY_LICENSES/`](tools/autoflash/THIRD_PARTY_LICENSES/).

### Rust crates (firmware)

The firmware and the tools are built from Rust crates that Cargo fetches
under their own licences (all permissive: MIT, Apache-2.0, BSD, ISC,
Zlib, Unicode-3.0; see `Cargo.lock`). `cargo about` or `cargo license`
lists them for a firmware release.

## Images and fonts

| Part | Where | Licence |
| --- | --- | --- |
| The Rust logo, by the Rust Foundation, from `rust-lang/rust-artwork`, unchanged except for its size | `assets/header.png` | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Rust and the Rust logo are trademarks of the Rust Foundation; this project is not affiliated with or endorsed by the Rust Project. |
| The ERNI name, logo and coat of arms | `assets/header.png`, `docs/face_id/presentation/assets/` | Trademarks of ERNI, used with its permission; **not** covered by this repository's licence. |
| Source Sans 3 and Source Code Pro, Copyright Adobe, Reserved Font Name "Source" | `docs/face_id/presentation/assets/*.woff2`, embedded in `face-id.html` and `face-id.pdf` | SIL Open Font License 1.1: [`OFL.txt`](docs/face_id/presentation/assets/OFL.txt) |
| The portrait of astronaut Jessica Watkins, NASA | `crates/vision/tests/fixtures/face_*.jpg`, `docs/face_id/presentation/assets/face_*` | Public domain (work of the United States government) |

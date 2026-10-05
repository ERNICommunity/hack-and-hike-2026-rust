# Model files

The files in this folder are committed as they were downloaded. To get
them again:

```bash
curl -L -o face_detection_yunet_2023mar.onnx \
  https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
curl -L -o face_detection_yunet_2026may.onnx \
  https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2026may.onnx
curl -L -o human_face_feat_mfn_s8_v1.espdl \
  https://github.com/espressif/esp-dl/raw/master/models/human_face_recognition/models/s3/human_face_feat_mfn_s8_v1.espdl
```

| File | What | Licence |
| --- | --- | --- |
| `face_detection_yunet_2023mar.onnx` | YuNet face detector, fixed 640x640 input | MIT, Copyright (c) 2020 Shiqi Yu ([`LICENSE-YuNet`](LICENSE-YuNet)) |
| `face_detection_yunet_2026may.onnx` | YuNet face detector, dynamic input size | MIT, Copyright (c) 2020 Shiqi Yu ([`LICENSE-YuNet`](LICENSE-YuNet)) |
| `human_face_feat_mfn_s8_v1.espdl` | MFN_S8_V1 face embedding (a MobileFaceNet), 112x112 input, `int8` for the ESP32-S3 | MIT, Copyright (c) 2021 Espressif Systems (Shanghai) Co., Ltd. ([`LICENSE-MFN_S8_V1`](LICENSE-MFN_S8_V1)) |

YuNet comes from OpenCV Zoo
(`models/face_detection_yunet`), MFN_S8_V1 from Espressif's ESP-DL
(`models/human_face_recognition`); each licence file above is the one in
that folder. Espressif does not say what MFN_S8_V1 was trained on.

The `.espdl` file is a FlatBuffer by ESP-DL's schema
(`esp-dl/fbs_loader/espdl.fbs`, MIT); `src/espdl.rs` reads it.

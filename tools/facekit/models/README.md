# Model files

The ONNX files in this folder are committed as they were downloaded. To
get them again:

```bash
curl -L -o face_detection_yunet_2023mar.onnx \
  https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx
curl -L -o face_detection_yunet_2026may.onnx \
  https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2026may.onnx
curl -L -o edgeface_xxs.onnx \
  https://github.com/yakhyo/edgeface-onnx/releases/download/weights/edgeface_xxs.onnx
```

| File | What | Licence |
| --- | --- | --- |
| `face_detection_yunet_2023mar.onnx` | YuNet face detector, fixed 640x640 input | MIT (OpenCV Zoo) |
| `face_detection_yunet_2026may.onnx` | YuNet face detector, dynamic input size | MIT (OpenCV Zoo) |
| `edgeface_xxs.onnx` | EdgeFace-XXS face embedding, 112x112 input | CC BY-NC-SA 4.0 (Idiap), ONNX export by yakhyo |

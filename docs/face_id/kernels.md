# Operators and their kernels

Every ONNX operator that appears in either model, and how the firmware
handles it. "Kernel" means a hand-written function in `crates/vision`.
"Export" means `facekit export` resolves it once on the developer machine
and the firmware never sees it. The counts come from the inventories in this
folder.

## Kernels with arithmetic

| Operator | YuNet | EdgeFace | Kernel | Notes |
| --- | --- | --- | --- | --- |
| `Conv` k×k, group 1 | 1 (3x3 s2 stem) + 3x1x1 heads | 1 (4x4 s4 stem), 3 (2x2 s2 downsample), 1 (1x1 pos-embd projection: folded by export) | `conv2d` | Padding and stride from the attributes. |
| `Conv` 1x1, group 1 | 30 | (see MatMul) | `pointwise` | A matrix multiply per pixel. The SIMD step optimizes this one. |
| `Conv` depthwise (group = channels) | 19 (3x3) | 2 (3x3), 1 (5x5), 5 (7x7), 1 (9x9), 6 (3x3 split convs) | `depthwise` | Kernel size up to 9x9. |
| `MatMul` with a weight | – | 36 (MLP fc1/fc2, qkv, proj) | `pointwise` | Same kernel as a 1x1 conv on channels-last data. |
| `Gemm` | – | 1 (head, 168 -> 512) | `pointwise` | One pixel. |
| `Add` (bias or residual) | 2 (neck) | 66 | fused into the producing kernel or `add` | Residual adds and the `+ 1` of GELU. |
| `Mul` | – | 56 | fused | Layer scale (`gamma`), the GELU factors, attention temperature. |
| `Div` | – | 25 | fused | `x / sqrt(2)` of GELU, L2-normalization of q and k. |
| `Relu` | 15 | – | fused into `conv2d`/`pointwise`/`depthwise` output | |
| `Sigmoid` | 6 (head outputs) | – | `sigmoid` | On 126 anchors, f32. |
| `Erf` | – | 12 | `gelu` | The whole `x * 0.5 * (1 + erf(x / sqrt 2))` pattern becomes one GELU kernel; a 256-entry table in the int8 step. |
| `LayerNormalization` | – | 20 | `layer_norm` | Over channels, per pixel, epsilon 1e-6, f32. |
| `ReduceL2` (+ `Clip`, `Expand`) | – | 6 (+6, +6) | `l2_normalize` | Normalizes q and k over the token axis inside XCA. `Clip` is the epsilon clamp of the norm. |
| `Softmax` | – | 3 | `xca` | Cross-covariance attention: channels attend to channels, one C×C matrix per head. |
| `MaxPool` 2x2 s2 | 4 | – | `max_pool_2x2` | |
| `Resize` nearest x2 | 2 | – | `upsample_2x` | Pixel duplication, fused into the following `Add`. |
| `GlobalAveragePool` | – | 1 | `global_average` | |

## Data movement (an index calculation, no kernel)

| Operator | YuNet | EdgeFace | Handling |
| --- | --- | --- | --- |
| `Reshape`, `Transpose`, `Flatten`, `Squeeze`, `Unsqueeze` | 12, 12, –, –, – | 15, 45, 1, 9, 47 | The firmware stores every tensor channels-last (`[H][W][C]`), which is the layout XCA and the MLPs want. The exporter records, for each block, which axis order the weights expect, and the hand-written forward pass indexes accordingly. Nothing is transposed at run time. |
| `Concat`, `Split`, `Slice` | – | 22, 3, 17 | Channel splits of the SDTA blocks and the q/k/v split: views into the channel axis, no copy. |
| `Cast`, `Shape`, `Gather`, `ConstantOfShape`, `Constant` | – | 4, 33, 25, 1, 194 | Shape arithmetic of the PyTorch export. Constant at a fixed input size; resolved by export. |

## Resolved by export

| Operator | EdgeFace | Handling |
| --- | --- | --- |
| `Sin`, `Cos`, `CumSum`, `Not` | 2, 2, 2, 1 | The Fourier positional encoding of the three SDTA blocks. `facekit export` runs the subgraph once through tract (up to and including the `token_projection` conv) and stores the result as a constant `[H][W][C]` tensor that the block adds. |

## Not present

`PRelu`, `AveragePool`, `Pow`, `Sqrt`, `Exp`, `Where`, `Equal`, `Identity`,
`Tile`, `Range` are in facekit's allowed list for safety but appear in
neither model. No unexpected operator appeared.

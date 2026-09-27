# models/edgeface_xxs.onnx
ir_version 8, producer pytorch 2.8.0
opset  17

## Inputs
input: f32[batch_size,3,112,112]

## Outputs
embedding: f32[batch_size,512]

## Nodes (681)
/model/stem/stem.0/Conv          Conv                 in=["input", "model.stem.0.weight", "model.stem.0.bias"] out=["/model/stem/stem.0/Conv_output_0"] weights=model.stem.0.weight[24, 3, 4, 4], model.stem.0.bias[24] dilations=[1, 1] group=1 kernel_shape=[4, 4] pads=[0, 0, 0, 0] strides=[4, 4]
/model/stem/stem.1/Transpose     Transpose            in=["/model/stem/stem.0/Conv_output_0"] out=["/model/stem/stem.1/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stem/stem.1/LayerNormalization LayerNormalization   in=["/model/stem/stem.1/Transpose_output_0", "model.stem.1.weight", "model.stem.1.bias"] out=["/model/stem/stem.1/LayerNormalization_output_0"] weights=model.stem.1.weight[24], model.stem.1.bias[24] axis=-1 epsilon=0.000001
/model/stem/stem.1/Transpose_1   Transpose            in=["/model/stem/stem.1/LayerNormalization_output_0"] out=["/model/stem/stem.1/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.0/blocks/blocks.0/conv_dw/Conv Conv                 in=["/model/stem/stem.1/Transpose_1_output_0", "model.stages.0.blocks.0.conv_dw.weight", "model.stages.0.blocks.0.conv_dw.bias"] out=["/model/stages/stages.0/blocks/blocks.0/conv_dw/Conv_output_0"] weights=model.stages.0.blocks.0.conv_dw.weight[24, 1, 3, 3], model.stages.0.blocks.0.conv_dw.bias[24] dilations=[1, 1] group=24 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.0/blocks/blocks.0/Transpose Transpose            in=["/model/stages/stages.0/blocks/blocks.0/conv_dw/Conv_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.0/blocks/blocks.0/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.0/blocks/blocks.0/Transpose_output_0", "model.stages.0.blocks.0.norm.weight", "model.stages.0.blocks.0.norm.bias"] out=["/model/stages/stages.0/blocks/blocks.0/norm/LayerNormalization_output_0"] weights=model.stages.0.blocks.0.norm.weight[24], model.stages.0.blocks.0.norm.bias[24] axis=-1 epsilon=0.000001
/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.0/blocks/blocks.0/norm/LayerNormalization_output_0", "onnx::MatMul_926"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_926[24, 96]
/model/stages/stages.0/blocks/blocks.0/mlp/fc1/Add Add                  in=["model.stages.0.blocks.0.mlp.fc1.bias", "/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/fc1/Add_output_0"] weights=model.stages.0.blocks.0.mlp.fc1.bias[96]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Div Div                  in=["/model/stages/stages.0/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Div_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Erf Erf                  in=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Div_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Erf_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_1_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Add Add                  in=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Erf_output_0", "/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Add_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul Mul                  in=["/model/stages/stages.0/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.0/blocks/blocks.0/mlp/act/Add_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_2_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul_output_0", "/model/stages/stages.0/blocks/blocks.0/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul_1_output_0"]
/model/stages/stages.0/blocks/blocks.0/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.0/blocks/blocks.0/mlp/act/Mul_1_output_0", "onnx::MatMul_927"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_927[96, 24]
/model/stages/stages.0/blocks/blocks.0/mlp/fc2/Add Add                  in=["model.stages.0.blocks.0.mlp.fc2.bias", "/model/stages/stages.0/blocks/blocks.0/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/mlp/fc2/Add_output_0"] weights=model.stages.0.blocks.0.mlp.fc2.bias[24]
/model/stages/stages.0/blocks/blocks.0/Mul Mul                  in=["model.stages.0.blocks.0.gamma", "/model/stages/stages.0/blocks/blocks.0/mlp/fc2/Add_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/Mul_output_0"] weights=model.stages.0.blocks.0.gamma[24]
/model/stages/stages.0/blocks/blocks.0/Transpose_1 Transpose            in=["/model/stages/stages.0/blocks/blocks.0/Mul_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.0/blocks/blocks.0/Add Add                  in=["/model/stem/stem.1/Transpose_1_output_0", "/model/stages/stages.0/blocks/blocks.0/Transpose_1_output_0"] out=["/model/stages/stages.0/blocks/blocks.0/Add_output_0"]
/model/stages/stages.0/blocks/blocks.1/conv_dw/Conv Conv                 in=["/model/stages/stages.0/blocks/blocks.0/Add_output_0", "model.stages.0.blocks.1.conv_dw.weight", "model.stages.0.blocks.1.conv_dw.bias"] out=["/model/stages/stages.0/blocks/blocks.1/conv_dw/Conv_output_0"] weights=model.stages.0.blocks.1.conv_dw.weight[24, 1, 3, 3], model.stages.0.blocks.1.conv_dw.bias[24] dilations=[1, 1] group=24 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.0/blocks/blocks.1/Transpose Transpose            in=["/model/stages/stages.0/blocks/blocks.1/conv_dw/Conv_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.0/blocks/blocks.1/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.0/blocks/blocks.1/Transpose_output_0", "model.stages.0.blocks.1.norm.weight", "model.stages.0.blocks.1.norm.bias"] out=["/model/stages/stages.0/blocks/blocks.1/norm/LayerNormalization_output_0"] weights=model.stages.0.blocks.1.norm.weight[24], model.stages.0.blocks.1.norm.bias[24] axis=-1 epsilon=0.000001
/model/stages/stages.0/blocks/blocks.1/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.0/blocks/blocks.1/norm/LayerNormalization_output_0", "onnx::MatMul_928"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_928[24, 96]
/model/stages/stages.0/blocks/blocks.1/mlp/fc1/Add Add                  in=["model.stages.0.blocks.1.mlp.fc1.bias", "/model/stages/stages.0/blocks/blocks.1/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/fc1/Add_output_0"] weights=model.stages.0.blocks.1.mlp.fc1.bias[96]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Div Div                  in=["/model/stages/stages.0/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Div_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Erf Erf                  in=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Div_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Erf_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_1_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Add Add                  in=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Erf_output_0", "/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Add_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul Mul                  in=["/model/stages/stages.0/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.0/blocks/blocks.1/mlp/act/Add_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_2_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul_output_0", "/model/stages/stages.0/blocks/blocks.1/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul_1_output_0"]
/model/stages/stages.0/blocks/blocks.1/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.0/blocks/blocks.1/mlp/act/Mul_1_output_0", "onnx::MatMul_929"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_929[96, 24]
/model/stages/stages.0/blocks/blocks.1/mlp/fc2/Add Add                  in=["model.stages.0.blocks.1.mlp.fc2.bias", "/model/stages/stages.0/blocks/blocks.1/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/mlp/fc2/Add_output_0"] weights=model.stages.0.blocks.1.mlp.fc2.bias[24]
/model/stages/stages.0/blocks/blocks.1/Mul Mul                  in=["model.stages.0.blocks.1.gamma", "/model/stages/stages.0/blocks/blocks.1/mlp/fc2/Add_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/Mul_output_0"] weights=model.stages.0.blocks.1.gamma[24]
/model/stages/stages.0/blocks/blocks.1/Transpose_1 Transpose            in=["/model/stages/stages.0/blocks/blocks.1/Mul_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.0/blocks/blocks.1/Add Add                  in=["/model/stages/stages.0/blocks/blocks.0/Add_output_0", "/model/stages/stages.0/blocks/blocks.1/Transpose_1_output_0"] out=["/model/stages/stages.0/blocks/blocks.1/Add_output_0"]
/model/stages/stages.1/downsample/downsample.0/Transpose Transpose            in=["/model/stages/stages.0/blocks/blocks.1/Add_output_0"] out=["/model/stages/stages.1/downsample/downsample.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.1/downsample/downsample.0/LayerNormalization LayerNormalization   in=["/model/stages/stages.1/downsample/downsample.0/Transpose_output_0", "model.stages.1.downsample.0.weight", "model.stages.1.downsample.0.bias"] out=["/model/stages/stages.1/downsample/downsample.0/LayerNormalization_output_0"] weights=model.stages.1.downsample.0.weight[24], model.stages.1.downsample.0.bias[24] axis=-1 epsilon=0.000001
/model/stages/stages.1/downsample/downsample.0/Transpose_1 Transpose            in=["/model/stages/stages.1/downsample/downsample.0/LayerNormalization_output_0"] out=["/model/stages/stages.1/downsample/downsample.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.1/downsample/downsample.1/Conv Conv                 in=["/model/stages/stages.1/downsample/downsample.0/Transpose_1_output_0", "model.stages.1.downsample.1.weight", "model.stages.1.downsample.1.bias"] out=["/model/stages/stages.1/downsample/downsample.1/Conv_output_0"] weights=model.stages.1.downsample.1.weight[48, 24, 2, 2], model.stages.1.downsample.1.bias[48] dilations=[1, 1] group=1 kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
/model/stages/stages.1/blocks/blocks.0/conv_dw/Conv Conv                 in=["/model/stages/stages.1/downsample/downsample.1/Conv_output_0", "model.stages.1.blocks.0.conv_dw.weight", "model.stages.1.blocks.0.conv_dw.bias"] out=["/model/stages/stages.1/blocks/blocks.0/conv_dw/Conv_output_0"] weights=model.stages.1.blocks.0.conv_dw.weight[48, 1, 5, 5], model.stages.1.blocks.0.conv_dw.bias[48] dilations=[1, 1] group=48 kernel_shape=[5, 5] pads=[2, 2, 2, 2] strides=[1, 1]
/model/stages/stages.1/blocks/blocks.0/Transpose Transpose            in=["/model/stages/stages.1/blocks/blocks.0/conv_dw/Conv_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.1/blocks/blocks.0/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.1/blocks/blocks.0/Transpose_output_0", "model.stages.1.blocks.0.norm.weight", "model.stages.1.blocks.0.norm.bias"] out=["/model/stages/stages.1/blocks/blocks.0/norm/LayerNormalization_output_0"] weights=model.stages.1.blocks.0.norm.weight[48], model.stages.1.blocks.0.norm.bias[48] axis=-1 epsilon=0.000001
/model/stages/stages.1/blocks/blocks.0/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.0/norm/LayerNormalization_output_0", "onnx::MatMul_930"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_930[48, 192]
/model/stages/stages.1/blocks/blocks.0/mlp/fc1/Add Add                  in=["model.stages.1.blocks.0.mlp.fc1.bias", "/model/stages/stages.1/blocks/blocks.0/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/fc1/Add_output_0"] weights=model.stages.1.blocks.0.mlp.fc1.bias[192]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Div Div                  in=["/model/stages/stages.1/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Div_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Erf Erf                  in=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Div_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Erf_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_1_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Add Add                  in=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Erf_output_0", "/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Add_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul Mul                  in=["/model/stages/stages.1/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.1/blocks/blocks.0/mlp/act/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_2_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul_output_0", "/model/stages/stages.1/blocks/blocks.0/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul_1_output_0"]
/model/stages/stages.1/blocks/blocks.0/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.0/mlp/act/Mul_1_output_0", "onnx::MatMul_931"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_931[192, 48]
/model/stages/stages.1/blocks/blocks.0/mlp/fc2/Add Add                  in=["model.stages.1.blocks.0.mlp.fc2.bias", "/model/stages/stages.1/blocks/blocks.0/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/mlp/fc2/Add_output_0"] weights=model.stages.1.blocks.0.mlp.fc2.bias[48]
/model/stages/stages.1/blocks/blocks.0/Mul Mul                  in=["model.stages.1.blocks.0.gamma", "/model/stages/stages.1/blocks/blocks.0/mlp/fc2/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/Mul_output_0"] weights=model.stages.1.blocks.0.gamma[48]
/model/stages/stages.1/blocks/blocks.0/Transpose_1 Transpose            in=["/model/stages/stages.1/blocks/blocks.0/Mul_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.1/blocks/blocks.0/Add Add                  in=["/model/stages/stages.1/downsample/downsample.1/Conv_output_0", "/model/stages/stages.1/blocks/blocks.0/Transpose_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.0/Add_output_0"]
/model/stages/stages.1/blocks/blocks.1/Shape Shape                in=["/model/stages/stages.1/blocks/blocks.0/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Constant_1 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_2 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/Add Add                  in=["/model/stages/stages.1/blocks/blocks.1/Gather_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Add_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_3 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/Div Div                  in=["/model/stages/stages.1/blocks/blocks.1/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Div_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_4 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/Mul Mul                  in=["/model/stages/stages.1/blocks/blocks.1/Div_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Mul_output_0"]
/model/stages/stages.1/blocks/blocks.1/Slice Slice                in=["/model/stages/stages.1/blocks/blocks.0/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_1_output_0", "/model/stages/stages.1/blocks/blocks.1/Mul_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Slice_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_5 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/Mul_1 Mul                  in=["/model/stages/stages.1/blocks/blocks.1/Div_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_5_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Mul_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/Slice_1 Slice                in=["/model/stages/stages.1/blocks/blocks.0/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/Mul_output_0", "/model/stages/stages.1/blocks/blocks.1/Mul_1_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Slice_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/convs.0/Conv Conv                 in=["/model/stages/stages.1/blocks/blocks.1/Slice_output_0", "model.stages.1.blocks.1.convs.0.weight", "model.stages.1.blocks.1.convs.0.bias"] out=["/model/stages/stages.1/blocks/blocks.1/convs.0/Conv_output_0"] weights=model.stages.1.blocks.1.convs.0.weight[24, 1, 3, 3], model.stages.1.blocks.1.convs.0.bias[24] dilations=[1, 1] group=24 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.1/blocks/blocks.1/Concat Concat               in=["/model/stages/stages.1/blocks/blocks.1/convs.0/Conv_output_0", "/model/stages/stages.1/blocks/blocks.1/Slice_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0"] axis=1
/model/stages/stages.1/blocks/blocks.1/Shape_1 Shape                in=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_6 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_6_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather_1 Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_1_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_6_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_1_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Shape_2 Shape                in=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_7 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_7_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather_2 Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_2_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_7_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_2_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Shape_3 Shape                in=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_8 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_8_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather_3 Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_3_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_8_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_3_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Shape_4 Shape                in=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_9 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_9_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather_4 Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_4_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_9_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_4_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Mul_2 Mul                  in=["/model/stages/stages.1/blocks/blocks.1/Gather_3_output_0", "/model/stages/stages.1/blocks/blocks.1/Gather_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Mul_2_output_0"]
Constant_285                     Constant             in=[] out=["onnx::Unsqueeze_262"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_262"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_output_0"]
Constant_287                     Constant             in=[] out=["onnx::Unsqueeze_264"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_2_output_0", "onnx::Unsqueeze_264"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_1_output_0"]
Constant_289                     Constant             in=[] out=["onnx::Unsqueeze_266"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Mul_2_output_0", "onnx::Unsqueeze_266"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/Concat_1 Concat               in=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_1_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Concat_1_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Reshape Reshape              in=["/model/stages/stages.1/blocks/blocks.1/Concat_output_0", "/model/stages/stages.1/blocks/blocks.1/Concat_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Reshape_output_0"]
/model/stages/stages.1/blocks/blocks.1/Transpose Transpose            in=["/model/stages/stages.1/blocks/blocks.1/Reshape_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Transpose_output_0"] perm=[0, 2, 1]
Constant_294                     Constant             in=[] out=["onnx::Unsqueeze_271"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_271"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_output_0"]
Constant_296                     Constant             in=[] out=["onnx::Unsqueeze_273"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_3_output_0", "onnx::Unsqueeze_273"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_1_output_0"]
Constant_298                     Constant             in=[] out=["onnx::Unsqueeze_275"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_4_output_0", "onnx::Unsqueeze_275"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/pos_embd/ConstantOfShape ConstantOfShape      in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/ConstantOfShape_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast Cast                 in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/ConstantOfShape_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Not Not                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Not_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_1 Cast                 in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Not_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum CumSum               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_1 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_2 Cast                 in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Not_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_1 CumSum               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_2_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_2 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_3 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_4 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_5 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_3_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_4_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_2_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_5_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_6 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_6_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Add Add                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_6_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Add_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Div Div                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_7 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_7_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul Mul                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_7_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_8 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_8_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_9 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_9_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_10 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_10_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_11 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_11_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_1 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_9_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_10_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_8_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_11_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_12 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_12_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Add_1 Add                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_12_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Add_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_1 Div                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/CumSum_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Add_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_13 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_13_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul_1 Mul                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_13_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_14 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_14_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_14_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_15 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_15_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_2 Div                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_3_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_15_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_16 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_16_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Mul_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_16_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_17 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_17_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_3 Div                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_4_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_17_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_18 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_18_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_19 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_19_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_20 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_20_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_21 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_21_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_2 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_2_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_19_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_20_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_18_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_21_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin Sin                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_22 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_22_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_23 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_23_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_24 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_24_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_25 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_25_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_3 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_2_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_23_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_24_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_22_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_25_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos Cos                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_26 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_26_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_5 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_26_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_27 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_27_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_6 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_27_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_6_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_1 Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_5_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_6_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_1_output_0"] axis=4
/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape Shape                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_28 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_28_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_29 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_29_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_30 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_30_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_4 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_29_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_30_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_28_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_31 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_31_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_2 Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_4_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_31_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_2_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape Reshape              in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_32 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_32_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_33 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_33_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_34 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_34_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_35 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_35_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_5 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_3_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_33_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_34_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_32_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_35_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin_1 Sin                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_5_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_36 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_36_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_37 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_37_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_38 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_38_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_39 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_39_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_6 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Div_3_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_37_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_38_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_36_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_39_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_6_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos_1 Cos                  in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_6_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_40 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_40_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_7 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Sin_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_40_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_7_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_41 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_41_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_8 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cos_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_41_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_8_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_3 Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_7_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Unsqueeze_8_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_3_output_0"] axis=4
/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape_1 Shape                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_42 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_42_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_43 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_43_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_44 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_44_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_7 Slice                in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Shape_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_43_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_44_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_42_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_7_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_45 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_45_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_4 Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Slice_7_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Constant_45_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_4_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape_1 Reshape              in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_3_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_5 Concat               in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape_1_output_0", "/model/stages/stages.1/blocks/blocks.1/pos_embd/Reshape_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_5_output_0"] axis=3
/model/stages/stages.1/blocks/blocks.1/pos_embd/Transpose Transpose            in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Concat_5_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Transpose_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_3 Cast                 in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Transpose_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/pos_embd/token_projection/Conv Conv                 in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/Cast_3_output_0", "model.stages.1.blocks.1.pos_embd.token_projection.weight", "model.stages.1.blocks.1.pos_embd.token_projection.bias"] out=["/model/stages/stages.1/blocks/blocks.1/pos_embd/token_projection/Conv_output_0"] weights=model.stages.1.blocks.1.pos_embd.token_projection.weight[48, 64, 1, 1], model.stages.1.blocks.1.pos_embd.token_projection.bias[48] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
/model/stages/stages.1/blocks/blocks.1/Shape_5 Shape                in=["/model/stages/stages.1/blocks/blocks.1/Transpose_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Shape_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_10 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_10_output_0"]
/model/stages/stages.1/blocks/blocks.1/Gather_5 Gather               in=["/model/stages/stages.1/blocks/blocks.1/Shape_5_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_10_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Gather_5_output_0"] axis=0
Constant_395                     Constant             in=[] out=["onnx::Unsqueeze_399"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_399"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/Constant_11 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/Constant_11_output_0"]
Constant_398                     Constant             in=[] out=["onnx::Unsqueeze_403"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_5_output_0", "onnx::Unsqueeze_403"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/Concat_2 Concat               in=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_3_output_0", "/model/stages/stages.1/blocks/blocks.1/Constant_11_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Concat_2_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Reshape_1 Reshape              in=["/model/stages/stages.1/blocks/blocks.1/pos_embd/token_projection/Conv_output_0", "/model/stages/stages.1/blocks/blocks.1/Concat_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Reshape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/Transpose_1 Transpose            in=["/model/stages/stages.1/blocks/blocks.1/Reshape_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Transpose_1_output_0"] perm=[0, 2, 1]
/model/stages/stages.1/blocks/blocks.1/Add_1 Add                  in=["/model/stages/stages.1/blocks/blocks.1/Transpose_output_0", "/model/stages/stages.1/blocks/blocks.1/Transpose_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Add_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization LayerNormalization   in=["/model/stages/stages.1/blocks/blocks.1/Add_1_output_0", "model.stages.1.blocks.1.norm_xca.weight", "model.stages.1.blocks.1.norm_xca.bias"] out=["/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] weights=model.stages.1.blocks.1.norm_xca.weight[48], model.stages.1.blocks.1.norm_xca.bias[48] axis=-1 epsilon=0.000001
/model/stages/stages.1/blocks/blocks.1/xca/Shape Shape                in=["/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Gather Gather               in=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/Shape_1 Shape                in=["/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_1 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Gather_1 Gather               in=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_1_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_1_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/Shape_2 Shape                in=["/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_2 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Gather_2 Gather               in=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_2_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_2_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/qkv/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.1/norm_xca/LayerNormalization_output_0", "onnx::MatMul_957"] out=["/model/stages/stages.1/blocks/blocks.1/xca/qkv/MatMul_output_0"] weights=onnx::MatMul_957[48, 144]
/model/stages/stages.1/blocks/blocks.1/xca/qkv/Add Add                  in=["model.stages.1.blocks.1.xca.qkv.bias", "/model/stages/stages.1/blocks/blocks.1/xca/qkv/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/qkv/Add_output_0"] weights=model.stages.1.blocks.1.xca.qkv.bias[144]
Constant_416                     Constant             in=[] out=["onnx::Unsqueeze_422"]
/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_output_0", "onnx::Unsqueeze_422"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_output_0"]
Constant_418                     Constant             in=[] out=["onnx::Unsqueeze_424"]
/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_1_output_0", "onnx::Unsqueeze_424"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_3 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_4 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_5 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_5_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Concat Concat               in=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_1_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_3_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_4_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_5_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Concat_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/Reshape Reshape              in=["/model/stages/stages.1/blocks/blocks.1/xca/qkv/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Concat_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Reshape_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Transpose Transpose            in=["/model/stages/stages.1/blocks/blocks.1/xca/Reshape_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Transpose_output_0"] perm=[2, 0, 3, 4, 1]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_6 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_6_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Split Split                in=["/model/stages/stages.1/blocks/blocks.1/xca/Transpose_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_6_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Split_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Split_output_1", "/model/stages/stages.1/blocks/blocks.1/xca/Split_output_2"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/Constant_7 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_7_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Squeeze Squeeze              in=["/model/stages/stages.1/blocks/blocks.1/xca/Split_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_7_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_8 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_8_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_1 Squeeze              in=["/model/stages/stages.1/blocks/blocks.1/xca/Split_output_1", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_8_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Constant_9 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_9_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_2 Squeeze              in=["/model/stages/stages.1/blocks/blocks.1/xca/Split_output_2", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_9_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2 ReduceL2             in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2_output_0"] axes=[-1] keepdims=1
/model/stages/stages.1/blocks/blocks.1/xca/Constant_10 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_10_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Clip Clip                 in=["/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_10_output_0", ""] out=["/model/stages/stages.1/blocks/blocks.1/xca/Clip_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Shape_3 Shape                in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_3_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Expand Expand               in=["/model/stages/stages.1/blocks/blocks.1/xca/Clip_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Shape_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Expand_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Div Div                  in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Expand_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Div_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2_1 ReduceL2             in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2_1_output_0"] axes=[-1] keepdims=1
/model/stages/stages.1/blocks/blocks.1/xca/Constant_11 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/xca/Constant_11_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Clip_1 Clip                 in=["/model/stages/stages.1/blocks/blocks.1/xca/ReduceL2_1_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Constant_11_output_0", ""] out=["/model/stages/stages.1/blocks/blocks.1/xca/Clip_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Shape_4 Shape                in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Shape_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Expand_1 Expand               in=["/model/stages/stages.1/blocks/blocks.1/xca/Clip_1_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Shape_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Expand_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Div_1 Div                  in=["/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_1_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Expand_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Div_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Transpose_1 Transpose            in=["/model/stages/stages.1/blocks/blocks.1/xca/Div_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Transpose_1_output_0"] perm=[0, 1, 3, 2]
/model/stages/stages.1/blocks/blocks.1/xca/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.1/xca/Div_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Transpose_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/MatMul_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Mul Mul                  in=["/model/stages/stages.1/blocks/blocks.1/xca/MatMul_output_0", "model.stages.1.blocks.1.xca.temperature"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Mul_output_0"] weights=model.stages.1.blocks.1.xca.temperature[4, 1, 1]
/model/stages/stages.1/blocks/blocks.1/xca/Softmax Softmax              in=["/model/stages/stages.1/blocks/blocks.1/xca/Mul_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Softmax_output_0"] axis=-1
/model/stages/stages.1/blocks/blocks.1/xca/MatMul_1 MatMul               in=["/model/stages/stages.1/blocks/blocks.1/xca/Softmax_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Squeeze_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/MatMul_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Transpose_2 Transpose            in=["/model/stages/stages.1/blocks/blocks.1/xca/MatMul_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Transpose_2_output_0"] perm=[0, 3, 1, 2]
Constant_452                     Constant             in=[] out=["onnx::Unsqueeze_466"]
/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_output_0", "onnx::Unsqueeze_466"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_2_output_0"]
Constant_454                     Constant             in=[] out=["onnx::Unsqueeze_468"]
/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_1_output_0", "onnx::Unsqueeze_468"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_3_output_0"]
Constant_456                     Constant             in=[] out=["onnx::Unsqueeze_470"]
/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/xca/Gather_2_output_0", "onnx::Unsqueeze_470"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_4_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/Concat_1 Concat               in=["/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_2_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_3_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Unsqueeze_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Concat_1_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/xca/Reshape_1 Reshape              in=["/model/stages/stages.1/blocks/blocks.1/xca/Transpose_2_output_0", "/model/stages/stages.1/blocks/blocks.1/xca/Concat_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/Reshape_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/xca/proj/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.1/xca/Reshape_1_output_0", "onnx::MatMul_963"] out=["/model/stages/stages.1/blocks/blocks.1/xca/proj/MatMul_output_0"] weights=onnx::MatMul_963[48, 48]
/model/stages/stages.1/blocks/blocks.1/xca/proj/Add Add                  in=["model.stages.1.blocks.1.xca.proj.bias", "/model/stages/stages.1/blocks/blocks.1/xca/proj/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/xca/proj/Add_output_0"] weights=model.stages.1.blocks.1.xca.proj.bias[48]
/model/stages/stages.1/blocks/blocks.1/Mul_3 Mul                  in=["model.stages.1.blocks.1.gamma_xca", "/model/stages/stages.1/blocks/blocks.1/xca/proj/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Mul_3_output_0"] weights=model.stages.1.blocks.1.gamma_xca[48]
/model/stages/stages.1/blocks/blocks.1/Add_2 Add                  in=["/model/stages/stages.1/blocks/blocks.1/Add_1_output_0", "/model/stages/stages.1/blocks/blocks.1/Mul_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Add_2_output_0"]
Constant_464                     Constant             in=[] out=["onnx::Unsqueeze_479"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_5 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_479"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_5_output_0"]
Constant_466                     Constant             in=[] out=["onnx::Unsqueeze_481"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_6 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_3_output_0", "onnx::Unsqueeze_481"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_6_output_0"]
Constant_468                     Constant             in=[] out=["onnx::Unsqueeze_483"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_7 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_4_output_0", "onnx::Unsqueeze_483"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_7_output_0"]
Constant_470                     Constant             in=[] out=["onnx::Unsqueeze_485"]
/model/stages/stages.1/blocks/blocks.1/Unsqueeze_8 Unsqueeze            in=["/model/stages/stages.1/blocks/blocks.1/Gather_2_output_0", "onnx::Unsqueeze_485"] out=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_8_output_0"]
/model/stages/stages.1/blocks/blocks.1/Concat_3 Concat               in=["/model/stages/stages.1/blocks/blocks.1/Unsqueeze_5_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_6_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_7_output_0", "/model/stages/stages.1/blocks/blocks.1/Unsqueeze_8_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Concat_3_output_0"] axis=0
/model/stages/stages.1/blocks/blocks.1/Reshape_2 Reshape              in=["/model/stages/stages.1/blocks/blocks.1/Add_2_output_0", "/model/stages/stages.1/blocks/blocks.1/Concat_3_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Reshape_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.1/blocks/blocks.1/Reshape_2_output_0", "model.stages.1.blocks.1.norm.weight", "model.stages.1.blocks.1.norm.bias"] out=["/model/stages/stages.1/blocks/blocks.1/norm/LayerNormalization_output_0"] weights=model.stages.1.blocks.1.norm.weight[48], model.stages.1.blocks.1.norm.bias[48] axis=-1 epsilon=0.000001
/model/stages/stages.1/blocks/blocks.1/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.1/norm/LayerNormalization_output_0", "onnx::MatMul_964"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_964[48, 192]
/model/stages/stages.1/blocks/blocks.1/mlp/fc1/Add Add                  in=["model.stages.1.blocks.1.mlp.fc1.bias", "/model/stages/stages.1/blocks/blocks.1/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/fc1/Add_output_0"] weights=model.stages.1.blocks.1.mlp.fc1.bias[192]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Div Div                  in=["/model/stages/stages.1/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Div_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Erf Erf                  in=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Div_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Erf_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Add Add                  in=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Erf_output_0", "/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Add_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul Mul                  in=["/model/stages/stages.1/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/mlp/act/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_2_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul_output_0", "/model/stages/stages.1/blocks/blocks.1/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul_1_output_0"]
/model/stages/stages.1/blocks/blocks.1/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.1/blocks/blocks.1/mlp/act/Mul_1_output_0", "onnx::MatMul_965"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_965[192, 48]
/model/stages/stages.1/blocks/blocks.1/mlp/fc2/Add Add                  in=["model.stages.1.blocks.1.mlp.fc2.bias", "/model/stages/stages.1/blocks/blocks.1/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/mlp/fc2/Add_output_0"] weights=model.stages.1.blocks.1.mlp.fc2.bias[48]
/model/stages/stages.1/blocks/blocks.1/Mul_4 Mul                  in=["model.stages.1.blocks.1.gamma", "/model/stages/stages.1/blocks/blocks.1/mlp/fc2/Add_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Mul_4_output_0"] weights=model.stages.1.blocks.1.gamma[48]
/model/stages/stages.1/blocks/blocks.1/Transpose_2 Transpose            in=["/model/stages/stages.1/blocks/blocks.1/Mul_4_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Transpose_2_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.1/blocks/blocks.1/Add_3 Add                  in=["/model/stages/stages.1/blocks/blocks.0/Add_output_0", "/model/stages/stages.1/blocks/blocks.1/Transpose_2_output_0"] out=["/model/stages/stages.1/blocks/blocks.1/Add_3_output_0"]
/model/stages/stages.2/downsample/downsample.0/Transpose Transpose            in=["/model/stages/stages.1/blocks/blocks.1/Add_3_output_0"] out=["/model/stages/stages.2/downsample/downsample.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/downsample/downsample.0/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/downsample/downsample.0/Transpose_output_0", "model.stages.2.downsample.0.weight", "model.stages.2.downsample.0.bias"] out=["/model/stages/stages.2/downsample/downsample.0/LayerNormalization_output_0"] weights=model.stages.2.downsample.0.weight[48], model.stages.2.downsample.0.bias[48] axis=-1 epsilon=0.000001
/model/stages/stages.2/downsample/downsample.0/Transpose_1 Transpose            in=["/model/stages/stages.2/downsample/downsample.0/LayerNormalization_output_0"] out=["/model/stages/stages.2/downsample/downsample.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/downsample/downsample.1/Conv Conv                 in=["/model/stages/stages.2/downsample/downsample.0/Transpose_1_output_0", "model.stages.2.downsample.1.weight", "model.stages.2.downsample.1.bias"] out=["/model/stages/stages.2/downsample/downsample.1/Conv_output_0"] weights=model.stages.2.downsample.1.weight[88, 48, 2, 2], model.stages.2.downsample.1.bias[88] dilations=[1, 1] group=1 kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
/model/stages/stages.2/blocks/blocks.0/conv_dw/Conv Conv                 in=["/model/stages/stages.2/downsample/downsample.1/Conv_output_0", "model.stages.2.blocks.0.conv_dw.weight", "model.stages.2.blocks.0.conv_dw.bias"] out=["/model/stages/stages.2/blocks/blocks.0/conv_dw/Conv_output_0"] weights=model.stages.2.blocks.0.conv_dw.weight[88, 1, 7, 7], model.stages.2.blocks.0.conv_dw.bias[88] dilations=[1, 1] group=88 kernel_shape=[7, 7] pads=[3, 3, 3, 3] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.0/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.0/conv_dw/Conv_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/blocks/blocks.0/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.0/Transpose_output_0", "model.stages.2.blocks.0.norm.weight", "model.stages.2.blocks.0.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.0/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.0.norm.weight[88], model.stages.2.blocks.0.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.0/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.0/norm/LayerNormalization_output_0", "onnx::MatMul_966"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_966[88, 352]
/model/stages/stages.2/blocks/blocks.0/mlp/fc1/Add Add                  in=["model.stages.2.blocks.0.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.0/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.0.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.0/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.0/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.0/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.0/mlp/act/Mul_1_output_0", "onnx::MatMul_967"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_967[352, 88]
/model/stages/stages.2/blocks/blocks.0/mlp/fc2/Add Add                  in=["model.stages.2.blocks.0.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.0/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.0.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.0/Mul Mul                  in=["model.stages.2.blocks.0.gamma", "/model/stages/stages.2/blocks/blocks.0/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/Mul_output_0"] weights=model.stages.2.blocks.0.gamma[88]
/model/stages/stages.2/blocks/blocks.0/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.0/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.0/Add Add                  in=["/model/stages/stages.2/downsample/downsample.1/Conv_output_0", "/model/stages/stages.2/blocks/blocks.0/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.0/Add_output_0"]
/model/stages/stages.2/blocks/blocks.1/conv_dw/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.0/Add_output_0", "model.stages.2.blocks.1.conv_dw.weight", "model.stages.2.blocks.1.conv_dw.bias"] out=["/model/stages/stages.2/blocks/blocks.1/conv_dw/Conv_output_0"] weights=model.stages.2.blocks.1.conv_dw.weight[88, 1, 7, 7], model.stages.2.blocks.1.conv_dw.bias[88] dilations=[1, 1] group=88 kernel_shape=[7, 7] pads=[3, 3, 3, 3] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.1/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.1/conv_dw/Conv_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/blocks/blocks.1/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.1/Transpose_output_0", "model.stages.2.blocks.1.norm.weight", "model.stages.2.blocks.1.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.1/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.1.norm.weight[88], model.stages.2.blocks.1.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.1/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.1/norm/LayerNormalization_output_0", "onnx::MatMul_968"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_968[88, 352]
/model/stages/stages.2/blocks/blocks.1/mlp/fc1/Add Add                  in=["model.stages.2.blocks.1.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.1/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.1.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.1/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.1/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.1/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.1/mlp/act/Mul_1_output_0", "onnx::MatMul_969"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_969[352, 88]
/model/stages/stages.2/blocks/blocks.1/mlp/fc2/Add Add                  in=["model.stages.2.blocks.1.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.1/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.1.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.1/Mul Mul                  in=["model.stages.2.blocks.1.gamma", "/model/stages/stages.2/blocks/blocks.1/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/Mul_output_0"] weights=model.stages.2.blocks.1.gamma[88]
/model/stages/stages.2/blocks/blocks.1/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.1/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.1/Add Add                  in=["/model/stages/stages.2/blocks/blocks.0/Add_output_0", "/model/stages/stages.2/blocks/blocks.1/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.1/Add_output_0"]
/model/stages/stages.2/blocks/blocks.2/conv_dw/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.1/Add_output_0", "model.stages.2.blocks.2.conv_dw.weight", "model.stages.2.blocks.2.conv_dw.bias"] out=["/model/stages/stages.2/blocks/blocks.2/conv_dw/Conv_output_0"] weights=model.stages.2.blocks.2.conv_dw.weight[88, 1, 7, 7], model.stages.2.blocks.2.conv_dw.bias[88] dilations=[1, 1] group=88 kernel_shape=[7, 7] pads=[3, 3, 3, 3] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.2/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.2/conv_dw/Conv_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/blocks/blocks.2/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.2/Transpose_output_0", "model.stages.2.blocks.2.norm.weight", "model.stages.2.blocks.2.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.2/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.2.norm.weight[88], model.stages.2.blocks.2.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.2/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.2/norm/LayerNormalization_output_0", "onnx::MatMul_970"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_970[88, 352]
/model/stages/stages.2/blocks/blocks.2/mlp/fc1/Add Add                  in=["model.stages.2.blocks.2.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.2/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.2.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.2/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.2/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.2/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.2/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.2/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.2/mlp/act/Mul_1_output_0", "onnx::MatMul_971"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_971[352, 88]
/model/stages/stages.2/blocks/blocks.2/mlp/fc2/Add Add                  in=["model.stages.2.blocks.2.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.2/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.2.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.2/Mul Mul                  in=["model.stages.2.blocks.2.gamma", "/model/stages/stages.2/blocks/blocks.2/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/Mul_output_0"] weights=model.stages.2.blocks.2.gamma[88]
/model/stages/stages.2/blocks/blocks.2/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.2/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.2/Add Add                  in=["/model/stages/stages.2/blocks/blocks.1/Add_output_0", "/model/stages/stages.2/blocks/blocks.2/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.2/Add_output_0"]
/model/stages/stages.2/blocks/blocks.3/conv_dw/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.2/Add_output_0", "model.stages.2.blocks.3.conv_dw.weight", "model.stages.2.blocks.3.conv_dw.bias"] out=["/model/stages/stages.2/blocks/blocks.3/conv_dw/Conv_output_0"] weights=model.stages.2.blocks.3.conv_dw.weight[88, 1, 7, 7], model.stages.2.blocks.3.conv_dw.bias[88] dilations=[1, 1] group=88 kernel_shape=[7, 7] pads=[3, 3, 3, 3] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.3/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.3/conv_dw/Conv_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/blocks/blocks.3/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.3/Transpose_output_0", "model.stages.2.blocks.3.norm.weight", "model.stages.2.blocks.3.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.3/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.3.norm.weight[88], model.stages.2.blocks.3.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.3/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.3/norm/LayerNormalization_output_0", "onnx::MatMul_972"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_972[88, 352]
/model/stages/stages.2/blocks/blocks.3/mlp/fc1/Add Add                  in=["model.stages.2.blocks.3.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.3/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.3.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.3/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.3/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.3/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.3/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.3/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.3/mlp/act/Mul_1_output_0", "onnx::MatMul_973"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_973[352, 88]
/model/stages/stages.2/blocks/blocks.3/mlp/fc2/Add Add                  in=["model.stages.2.blocks.3.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.3/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.3.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.3/Mul Mul                  in=["model.stages.2.blocks.3.gamma", "/model/stages/stages.2/blocks/blocks.3/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/Mul_output_0"] weights=model.stages.2.blocks.3.gamma[88]
/model/stages/stages.2/blocks/blocks.3/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.3/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.3/Add Add                  in=["/model/stages/stages.2/blocks/blocks.2/Add_output_0", "/model/stages/stages.2/blocks/blocks.3/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.3/Add_output_0"]
/model/stages/stages.2/blocks/blocks.4/conv_dw/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.3/Add_output_0", "model.stages.2.blocks.4.conv_dw.weight", "model.stages.2.blocks.4.conv_dw.bias"] out=["/model/stages/stages.2/blocks/blocks.4/conv_dw/Conv_output_0"] weights=model.stages.2.blocks.4.conv_dw.weight[88, 1, 7, 7], model.stages.2.blocks.4.conv_dw.bias[88] dilations=[1, 1] group=88 kernel_shape=[7, 7] pads=[3, 3, 3, 3] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.4/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.4/conv_dw/Conv_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.2/blocks/blocks.4/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.4/Transpose_output_0", "model.stages.2.blocks.4.norm.weight", "model.stages.2.blocks.4.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.4/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.4.norm.weight[88], model.stages.2.blocks.4.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.4/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.4/norm/LayerNormalization_output_0", "onnx::MatMul_974"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_974[88, 352]
/model/stages/stages.2/blocks/blocks.4/mlp/fc1/Add Add                  in=["model.stages.2.blocks.4.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.4/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.4.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.4/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.4/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.4/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.4/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.4/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.4/mlp/act/Mul_1_output_0", "onnx::MatMul_975"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_975[352, 88]
/model/stages/stages.2/blocks/blocks.4/mlp/fc2/Add Add                  in=["model.stages.2.blocks.4.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.4/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.4.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.4/Mul Mul                  in=["model.stages.2.blocks.4.gamma", "/model/stages/stages.2/blocks/blocks.4/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/Mul_output_0"] weights=model.stages.2.blocks.4.gamma[88]
/model/stages/stages.2/blocks/blocks.4/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.4/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.4/Add Add                  in=["/model/stages/stages.2/blocks/blocks.3/Add_output_0", "/model/stages/stages.2/blocks/blocks.4/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.4/Add_output_0"]
/model/stages/stages.2/blocks/blocks.5/Shape Shape                in=["/model/stages/stages.2/blocks/blocks.4/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Shape_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.5/Gather Gather               in=["/model/stages/stages.2/blocks/blocks.5/Shape_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Gather_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/Add Add                  in=["/model/stages/stages.2/blocks/blocks.5/Gather_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Add_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_3 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_3_output_0"]
/model/stages/stages.2/blocks/blocks.5/Div Div                  in=["/model/stages/stages.2/blocks/blocks.5/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_3_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Div_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_4 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_4_output_0"]
/model/stages/stages.2/blocks/blocks.5/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.5/Div_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_4_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.5/Slice Slice                in=["/model/stages/stages.2/blocks/blocks.4/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_1_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Slice_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_5 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_5_output_0"]
/model/stages/stages.2/blocks/blocks.5/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.5/Div_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_5_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/Slice_1 Slice                in=["/model/stages/stages.2/blocks/blocks.4/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_1_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Slice_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_6 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_6_output_0"]
/model/stages/stages.2/blocks/blocks.5/Mul_2 Mul                  in=["/model/stages/stages.2/blocks/blocks.5/Div_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_6_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/Slice_2 Slice                in=["/model/stages/stages.2/blocks/blocks.4/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_1_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_2_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Slice_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/convs.0/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.5/Slice_output_0", "model.stages.2.blocks.5.convs.0.weight", "model.stages.2.blocks.5.convs.0.bias"] out=["/model/stages/stages.2/blocks/blocks.5/convs.0/Conv_output_0"] weights=model.stages.2.blocks.5.convs.0.weight[30, 1, 3, 3], model.stages.2.blocks.5.convs.0.bias[30] dilations=[1, 1] group=30 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.5/Add_1 Add                  in=["/model/stages/stages.2/blocks/blocks.5/convs.0/Conv_output_0", "/model/stages/stages.2/blocks/blocks.5/Slice_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Add_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/convs.1/Conv Conv                 in=["/model/stages/stages.2/blocks/blocks.5/Add_1_output_0", "model.stages.2.blocks.5.convs.1.weight", "model.stages.2.blocks.5.convs.1.bias"] out=["/model/stages/stages.2/blocks/blocks.5/convs.1/Conv_output_0"] weights=model.stages.2.blocks.5.convs.1.weight[30, 1, 3, 3], model.stages.2.blocks.5.convs.1.bias[30] dilations=[1, 1] group=30 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.2/blocks/blocks.5/Concat Concat               in=["/model/stages/stages.2/blocks/blocks.5/convs.0/Conv_output_0", "/model/stages/stages.2/blocks/blocks.5/convs.1/Conv_output_0", "/model/stages/stages.2/blocks/blocks.5/Slice_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0"] axis=1
/model/stages/stages.2/blocks/blocks.5/Shape_1 Shape                in=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Shape_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_7 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_7_output_0"]
/model/stages/stages.2/blocks/blocks.5/Gather_1 Gather               in=["/model/stages/stages.2/blocks/blocks.5/Shape_1_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_7_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Gather_1_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Shape_2 Shape                in=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Shape_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_8 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_8_output_0"]
/model/stages/stages.2/blocks/blocks.5/Gather_2 Gather               in=["/model/stages/stages.2/blocks/blocks.5/Shape_2_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_8_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Gather_2_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Shape_3 Shape                in=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Shape_3_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_9 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_9_output_0"]
/model/stages/stages.2/blocks/blocks.5/Gather_3 Gather               in=["/model/stages/stages.2/blocks/blocks.5/Shape_3_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_9_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Gather_3_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Shape_4 Shape                in=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Shape_4_output_0"]
/model/stages/stages.2/blocks/blocks.5/Constant_10 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/Constant_10_output_0"]
/model/stages/stages.2/blocks/blocks.5/Gather_4 Gather               in=["/model/stages/stages.2/blocks/blocks.5/Shape_4_output_0", "/model/stages/stages.2/blocks/blocks.5/Constant_10_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Gather_4_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Mul_3 Mul                  in=["/model/stages/stages.2/blocks/blocks.5/Gather_3_output_0", "/model/stages/stages.2/blocks/blocks.5/Gather_4_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_3_output_0"]
Constant_618                     Constant             in=[] out=["onnx::Unsqueeze_645"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_1_output_0", "onnx::Unsqueeze_645"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_output_0"]
Constant_620                     Constant             in=[] out=["onnx::Unsqueeze_647"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_2_output_0", "onnx::Unsqueeze_647"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_1_output_0"]
Constant_622                     Constant             in=[] out=["onnx::Unsqueeze_649"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Mul_3_output_0", "onnx::Unsqueeze_649"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/Concat_1 Concat               in=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_output_0", "/model/stages/stages.2/blocks/blocks.5/Unsqueeze_1_output_0", "/model/stages/stages.2/blocks/blocks.5/Unsqueeze_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Concat_1_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Reshape Reshape              in=["/model/stages/stages.2/blocks/blocks.5/Concat_output_0", "/model/stages/stages.2/blocks/blocks.5/Concat_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Reshape_output_0"]
/model/stages/stages.2/blocks/blocks.5/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.5/Reshape_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Transpose_output_0"] perm=[0, 2, 1]
/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.5/Transpose_output_0", "model.stages.2.blocks.5.norm_xca.weight", "model.stages.2.blocks.5.norm_xca.bias"] out=["/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization_output_0"] weights=model.stages.2.blocks.5.norm_xca.weight[88], model.stages.2.blocks.5.norm_xca.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.5/xca/Shape Shape                in=["/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Gather Gather               in=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/Shape_1 Shape                in=["/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Gather_1 Gather               in=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_1_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_1_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/Shape_2 Shape                in=["/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Gather_2 Gather               in=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_2_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_2_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/qkv/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.5/norm_xca/LayerNormalization_output_0", "onnx::MatMul_976"] out=["/model/stages/stages.2/blocks/blocks.5/xca/qkv/MatMul_output_0"] weights=onnx::MatMul_976[88, 264]
/model/stages/stages.2/blocks/blocks.5/xca/qkv/Add Add                  in=["model.stages.2.blocks.5.xca.qkv.bias", "/model/stages/stages.2/blocks/blocks.5/xca/qkv/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/qkv/Add_output_0"] weights=model.stages.2.blocks.5.xca.qkv.bias[264]
Constant_639                     Constant             in=[] out=["onnx::Unsqueeze_667"]
/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_output_0", "onnx::Unsqueeze_667"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_output_0"]
Constant_641                     Constant             in=[] out=["onnx::Unsqueeze_669"]
/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_1_output_0", "onnx::Unsqueeze_669"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_3 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_3_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_4 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_4_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_5 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_5_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Concat Concat               in=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_1_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_3_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_4_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_5_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Concat_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/Reshape Reshape              in=["/model/stages/stages.2/blocks/blocks.5/xca/qkv/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Concat_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Reshape_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.5/xca/Reshape_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Transpose_output_0"] perm=[2, 0, 3, 4, 1]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_6 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_6_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Split Split                in=["/model/stages/stages.2/blocks/blocks.5/xca/Transpose_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_6_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Split_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Split_output_1", "/model/stages/stages.2/blocks/blocks.5/xca/Split_output_2"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/Constant_7 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_7_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Squeeze Squeeze              in=["/model/stages/stages.2/blocks/blocks.5/xca/Split_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_7_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_8 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_8_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_1 Squeeze              in=["/model/stages/stages.2/blocks/blocks.5/xca/Split_output_1", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_8_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Constant_9 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_9_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_2 Squeeze              in=["/model/stages/stages.2/blocks/blocks.5/xca/Split_output_2", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_9_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2 ReduceL2             in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2_output_0"] axes=[-1] keepdims=1
/model/stages/stages.2/blocks/blocks.5/xca/Constant_10 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_10_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Clip Clip                 in=["/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_10_output_0", ""] out=["/model/stages/stages.2/blocks/blocks.5/xca/Clip_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Shape_3 Shape                in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_3_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Expand Expand               in=["/model/stages/stages.2/blocks/blocks.5/xca/Clip_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Shape_3_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Expand_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Div Div                  in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Expand_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Div_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2_1 ReduceL2             in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2_1_output_0"] axes=[-1] keepdims=1
/model/stages/stages.2/blocks/blocks.5/xca/Constant_11 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/xca/Constant_11_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Clip_1 Clip                 in=["/model/stages/stages.2/blocks/blocks.5/xca/ReduceL2_1_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Constant_11_output_0", ""] out=["/model/stages/stages.2/blocks/blocks.5/xca/Clip_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Shape_4 Shape                in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Shape_4_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Expand_1 Expand               in=["/model/stages/stages.2/blocks/blocks.5/xca/Clip_1_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Shape_4_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Expand_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Div_1 Div                  in=["/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_1_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Expand_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Div_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.5/xca/Div_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Transpose_1_output_0"] perm=[0, 1, 3, 2]
/model/stages/stages.2/blocks/blocks.5/xca/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.5/xca/Div_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/MatMul_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.5/xca/MatMul_output_0", "model.stages.2.blocks.5.xca.temperature"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Mul_output_0"] weights=model.stages.2.blocks.5.xca.temperature[4, 1, 1]
/model/stages/stages.2/blocks/blocks.5/xca/Softmax Softmax              in=["/model/stages/stages.2/blocks/blocks.5/xca/Mul_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Softmax_output_0"] axis=-1
/model/stages/stages.2/blocks/blocks.5/xca/MatMul_1 MatMul               in=["/model/stages/stages.2/blocks/blocks.5/xca/Softmax_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Squeeze_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/MatMul_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Transpose_2 Transpose            in=["/model/stages/stages.2/blocks/blocks.5/xca/MatMul_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Transpose_2_output_0"] perm=[0, 3, 1, 2]
Constant_675                     Constant             in=[] out=["onnx::Unsqueeze_710"]
/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_output_0", "onnx::Unsqueeze_710"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_2_output_0"]
Constant_677                     Constant             in=[] out=["onnx::Unsqueeze_712"]
/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_1_output_0", "onnx::Unsqueeze_712"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_3_output_0"]
Constant_679                     Constant             in=[] out=["onnx::Unsqueeze_714"]
/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/xca/Gather_2_output_0", "onnx::Unsqueeze_714"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_4_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/Concat_1 Concat               in=["/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_2_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_3_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Unsqueeze_4_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Concat_1_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/xca/Reshape_1 Reshape              in=["/model/stages/stages.2/blocks/blocks.5/xca/Transpose_2_output_0", "/model/stages/stages.2/blocks/blocks.5/xca/Concat_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/Reshape_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/xca/proj/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.5/xca/Reshape_1_output_0", "onnx::MatMul_982"] out=["/model/stages/stages.2/blocks/blocks.5/xca/proj/MatMul_output_0"] weights=onnx::MatMul_982[88, 88]
/model/stages/stages.2/blocks/blocks.5/xca/proj/Add Add                  in=["model.stages.2.blocks.5.xca.proj.bias", "/model/stages/stages.2/blocks/blocks.5/xca/proj/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/xca/proj/Add_output_0"] weights=model.stages.2.blocks.5.xca.proj.bias[88]
/model/stages/stages.2/blocks/blocks.5/Mul_4 Mul                  in=["model.stages.2.blocks.5.gamma_xca", "/model/stages/stages.2/blocks/blocks.5/xca/proj/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_4_output_0"] weights=model.stages.2.blocks.5.gamma_xca[88]
/model/stages/stages.2/blocks/blocks.5/Add_2 Add                  in=["/model/stages/stages.2/blocks/blocks.5/Transpose_output_0", "/model/stages/stages.2/blocks/blocks.5/Mul_4_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Add_2_output_0"]
Constant_687                     Constant             in=[] out=["onnx::Unsqueeze_723"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_1_output_0", "onnx::Unsqueeze_723"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_3_output_0"]
Constant_689                     Constant             in=[] out=["onnx::Unsqueeze_725"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_3_output_0", "onnx::Unsqueeze_725"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_4_output_0"]
Constant_691                     Constant             in=[] out=["onnx::Unsqueeze_727"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_5 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_4_output_0", "onnx::Unsqueeze_727"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_5_output_0"]
Constant_693                     Constant             in=[] out=["onnx::Unsqueeze_729"]
/model/stages/stages.2/blocks/blocks.5/Unsqueeze_6 Unsqueeze            in=["/model/stages/stages.2/blocks/blocks.5/Gather_2_output_0", "onnx::Unsqueeze_729"] out=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_6_output_0"]
/model/stages/stages.2/blocks/blocks.5/Concat_2 Concat               in=["/model/stages/stages.2/blocks/blocks.5/Unsqueeze_3_output_0", "/model/stages/stages.2/blocks/blocks.5/Unsqueeze_4_output_0", "/model/stages/stages.2/blocks/blocks.5/Unsqueeze_5_output_0", "/model/stages/stages.2/blocks/blocks.5/Unsqueeze_6_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Concat_2_output_0"] axis=0
/model/stages/stages.2/blocks/blocks.5/Reshape_1 Reshape              in=["/model/stages/stages.2/blocks/blocks.5/Add_2_output_0", "/model/stages/stages.2/blocks/blocks.5/Concat_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Reshape_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.2/blocks/blocks.5/Reshape_1_output_0", "model.stages.2.blocks.5.norm.weight", "model.stages.2.blocks.5.norm.bias"] out=["/model/stages/stages.2/blocks/blocks.5/norm/LayerNormalization_output_0"] weights=model.stages.2.blocks.5.norm.weight[88], model.stages.2.blocks.5.norm.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.2/blocks/blocks.5/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.5/norm/LayerNormalization_output_0", "onnx::MatMul_983"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_983[88, 352]
/model/stages/stages.2/blocks/blocks.5/mlp/fc1/Add Add                  in=["model.stages.2.blocks.5.mlp.fc1.bias", "/model/stages/stages.2/blocks/blocks.5/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/fc1/Add_output_0"] weights=model.stages.2.blocks.5.mlp.fc1.bias[352]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Div Div                  in=["/model/stages/stages.2/blocks/blocks.5/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Div_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Erf Erf                  in=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Div_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Erf_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Add Add                  in=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Erf_output_0", "/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Add_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul Mul                  in=["/model/stages/stages.2/blocks/blocks.5/mlp/fc1/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/mlp/act/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_2_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul_output_0", "/model/stages/stages.2/blocks/blocks.5/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul_1_output_0"]
/model/stages/stages.2/blocks/blocks.5/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.2/blocks/blocks.5/mlp/act/Mul_1_output_0", "onnx::MatMul_984"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_984[352, 88]
/model/stages/stages.2/blocks/blocks.5/mlp/fc2/Add Add                  in=["model.stages.2.blocks.5.mlp.fc2.bias", "/model/stages/stages.2/blocks/blocks.5/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/mlp/fc2/Add_output_0"] weights=model.stages.2.blocks.5.mlp.fc2.bias[88]
/model/stages/stages.2/blocks/blocks.5/Mul_5 Mul                  in=["model.stages.2.blocks.5.gamma", "/model/stages/stages.2/blocks/blocks.5/mlp/fc2/Add_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Mul_5_output_0"] weights=model.stages.2.blocks.5.gamma[88]
/model/stages/stages.2/blocks/blocks.5/Transpose_1 Transpose            in=["/model/stages/stages.2/blocks/blocks.5/Mul_5_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.2/blocks/blocks.5/Add_3 Add                  in=["/model/stages/stages.2/blocks/blocks.4/Add_output_0", "/model/stages/stages.2/blocks/blocks.5/Transpose_1_output_0"] out=["/model/stages/stages.2/blocks/blocks.5/Add_3_output_0"]
/model/stages/stages.3/downsample/downsample.0/Transpose Transpose            in=["/model/stages/stages.2/blocks/blocks.5/Add_3_output_0"] out=["/model/stages/stages.3/downsample/downsample.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.3/downsample/downsample.0/LayerNormalization LayerNormalization   in=["/model/stages/stages.3/downsample/downsample.0/Transpose_output_0", "model.stages.3.downsample.0.weight", "model.stages.3.downsample.0.bias"] out=["/model/stages/stages.3/downsample/downsample.0/LayerNormalization_output_0"] weights=model.stages.3.downsample.0.weight[88], model.stages.3.downsample.0.bias[88] axis=-1 epsilon=0.000001
/model/stages/stages.3/downsample/downsample.0/Transpose_1 Transpose            in=["/model/stages/stages.3/downsample/downsample.0/LayerNormalization_output_0"] out=["/model/stages/stages.3/downsample/downsample.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.3/downsample/downsample.1/Conv Conv                 in=["/model/stages/stages.3/downsample/downsample.0/Transpose_1_output_0", "model.stages.3.downsample.1.weight", "model.stages.3.downsample.1.bias"] out=["/model/stages/stages.3/downsample/downsample.1/Conv_output_0"] weights=model.stages.3.downsample.1.weight[168, 88, 2, 2], model.stages.3.downsample.1.bias[168] dilations=[1, 1] group=1 kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
/model/stages/stages.3/blocks/blocks.0/conv_dw/Conv Conv                 in=["/model/stages/stages.3/downsample/downsample.1/Conv_output_0", "model.stages.3.blocks.0.conv_dw.weight", "model.stages.3.blocks.0.conv_dw.bias"] out=["/model/stages/stages.3/blocks/blocks.0/conv_dw/Conv_output_0"] weights=model.stages.3.blocks.0.conv_dw.weight[168, 1, 9, 9], model.stages.3.blocks.0.conv_dw.bias[168] dilations=[1, 1] group=168 kernel_shape=[9, 9] pads=[4, 4, 4, 4] strides=[1, 1]
/model/stages/stages.3/blocks/blocks.0/Transpose Transpose            in=["/model/stages/stages.3/blocks/blocks.0/conv_dw/Conv_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/stages/stages.3/blocks/blocks.0/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.3/blocks/blocks.0/Transpose_output_0", "model.stages.3.blocks.0.norm.weight", "model.stages.3.blocks.0.norm.bias"] out=["/model/stages/stages.3/blocks/blocks.0/norm/LayerNormalization_output_0"] weights=model.stages.3.blocks.0.norm.weight[168], model.stages.3.blocks.0.norm.bias[168] axis=-1 epsilon=0.000001
/model/stages/stages.3/blocks/blocks.0/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.0/norm/LayerNormalization_output_0", "onnx::MatMul_985"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_985[168, 672]
/model/stages/stages.3/blocks/blocks.0/mlp/fc1/Add Add                  in=["model.stages.3.blocks.0.mlp.fc1.bias", "/model/stages/stages.3/blocks/blocks.0/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/fc1/Add_output_0"] weights=model.stages.3.blocks.0.mlp.fc1.bias[672]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Div Div                  in=["/model/stages/stages.3/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Div_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Erf Erf                  in=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Div_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Erf_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_1_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Add Add                  in=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Erf_output_0", "/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Add_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul Mul                  in=["/model/stages/stages.3/blocks/blocks.0/mlp/fc1/Add_output_0", "/model/stages/stages.3/blocks/blocks.0/mlp/act/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_2_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul_output_0", "/model/stages/stages.3/blocks/blocks.0/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul_1_output_0"]
/model/stages/stages.3/blocks/blocks.0/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.0/mlp/act/Mul_1_output_0", "onnx::MatMul_986"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_986[672, 168]
/model/stages/stages.3/blocks/blocks.0/mlp/fc2/Add Add                  in=["model.stages.3.blocks.0.mlp.fc2.bias", "/model/stages/stages.3/blocks/blocks.0/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/mlp/fc2/Add_output_0"] weights=model.stages.3.blocks.0.mlp.fc2.bias[168]
/model/stages/stages.3/blocks/blocks.0/Mul Mul                  in=["model.stages.3.blocks.0.gamma", "/model/stages/stages.3/blocks/blocks.0/mlp/fc2/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/Mul_output_0"] weights=model.stages.3.blocks.0.gamma[168]
/model/stages/stages.3/blocks/blocks.0/Transpose_1 Transpose            in=["/model/stages/stages.3/blocks/blocks.0/Mul_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.3/blocks/blocks.0/Add Add                  in=["/model/stages/stages.3/downsample/downsample.1/Conv_output_0", "/model/stages/stages.3/blocks/blocks.0/Transpose_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.0/Add_output_0"]
/model/stages/stages.3/blocks/blocks.1/Shape Shape                in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Shape_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_output_0"]
/model/stages/stages.3/blocks/blocks.1/Gather Gather               in=["/model/stages/stages.3/blocks/blocks.1/Shape_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Gather_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Constant_1 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_2 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/Add Add                  in=["/model/stages/stages.3/blocks/blocks.1/Gather_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Add_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_3 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/Div Div                  in=["/model/stages/stages.3/blocks/blocks.1/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_3_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Div_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_4 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_4_output_0"]
/model/stages/stages.3/blocks/blocks.1/Mul Mul                  in=["/model/stages/stages.3/blocks/blocks.1/Div_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_4_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_output_0"]
/model/stages/stages.3/blocks/blocks.1/Slice Slice                in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_1_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Slice_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_5 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_5_output_0"]
/model/stages/stages.3/blocks/blocks.1/Mul_1 Mul                  in=["/model/stages/stages.3/blocks/blocks.1/Div_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_5_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/Slice_1 Slice                in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_1_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Slice_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_6 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_6_output_0"]
/model/stages/stages.3/blocks/blocks.1/Mul_2 Mul                  in=["/model/stages/stages.3/blocks/blocks.1/Div_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_6_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/Slice_2 Slice                in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_1_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_2_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Slice_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_7 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_7_output_0"]
/model/stages/stages.3/blocks/blocks.1/Mul_3 Mul                  in=["/model/stages/stages.3/blocks/blocks.1/Div_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_7_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/Slice_3 Slice                in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_2_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_3_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Slice_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/convs.0/Conv Conv                 in=["/model/stages/stages.3/blocks/blocks.1/Slice_output_0", "model.stages.3.blocks.1.convs.0.weight", "model.stages.3.blocks.1.convs.0.bias"] out=["/model/stages/stages.3/blocks/blocks.1/convs.0/Conv_output_0"] weights=model.stages.3.blocks.1.convs.0.weight[42, 1, 3, 3], model.stages.3.blocks.1.convs.0.bias[42] dilations=[1, 1] group=42 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.3/blocks/blocks.1/Add_1 Add                  in=["/model/stages/stages.3/blocks/blocks.1/convs.0/Conv_output_0", "/model/stages/stages.3/blocks/blocks.1/Slice_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Add_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/convs.1/Conv Conv                 in=["/model/stages/stages.3/blocks/blocks.1/Add_1_output_0", "model.stages.3.blocks.1.convs.1.weight", "model.stages.3.blocks.1.convs.1.bias"] out=["/model/stages/stages.3/blocks/blocks.1/convs.1/Conv_output_0"] weights=model.stages.3.blocks.1.convs.1.weight[42, 1, 3, 3], model.stages.3.blocks.1.convs.1.bias[42] dilations=[1, 1] group=42 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.3/blocks/blocks.1/Add_2 Add                  in=["/model/stages/stages.3/blocks/blocks.1/convs.1/Conv_output_0", "/model/stages/stages.3/blocks/blocks.1/Slice_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Add_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/convs.2/Conv Conv                 in=["/model/stages/stages.3/blocks/blocks.1/Add_2_output_0", "model.stages.3.blocks.1.convs.2.weight", "model.stages.3.blocks.1.convs.2.bias"] out=["/model/stages/stages.3/blocks/blocks.1/convs.2/Conv_output_0"] weights=model.stages.3.blocks.1.convs.2.weight[42, 1, 3, 3], model.stages.3.blocks.1.convs.2.bias[42] dilations=[1, 1] group=42 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
/model/stages/stages.3/blocks/blocks.1/Concat Concat               in=["/model/stages/stages.3/blocks/blocks.1/convs.0/Conv_output_0", "/model/stages/stages.3/blocks/blocks.1/convs.1/Conv_output_0", "/model/stages/stages.3/blocks/blocks.1/convs.2/Conv_output_0", "/model/stages/stages.3/blocks/blocks.1/Slice_3_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0"] axis=1
/model/stages/stages.3/blocks/blocks.1/Shape_1 Shape                in=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Shape_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_8 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_8_output_0"]
/model/stages/stages.3/blocks/blocks.1/Gather_1 Gather               in=["/model/stages/stages.3/blocks/blocks.1/Shape_1_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_8_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Gather_1_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Shape_2 Shape                in=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Shape_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_9 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_9_output_0"]
/model/stages/stages.3/blocks/blocks.1/Gather_2 Gather               in=["/model/stages/stages.3/blocks/blocks.1/Shape_2_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_9_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Gather_2_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Shape_3 Shape                in=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Shape_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_10 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_10_output_0"]
/model/stages/stages.3/blocks/blocks.1/Gather_3 Gather               in=["/model/stages/stages.3/blocks/blocks.1/Shape_3_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_10_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Gather_3_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Shape_4 Shape                in=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Shape_4_output_0"]
/model/stages/stages.3/blocks/blocks.1/Constant_11 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/Constant_11_output_0"]
/model/stages/stages.3/blocks/blocks.1/Gather_4 Gather               in=["/model/stages/stages.3/blocks/blocks.1/Shape_4_output_0", "/model/stages/stages.3/blocks/blocks.1/Constant_11_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Gather_4_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Mul_4 Mul                  in=["/model/stages/stages.3/blocks/blocks.1/Gather_3_output_0", "/model/stages/stages.3/blocks/blocks.1/Gather_4_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_4_output_0"]
Constant_774                     Constant             in=[] out=["onnx::Unsqueeze_814"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_814"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_output_0"]
Constant_776                     Constant             in=[] out=["onnx::Unsqueeze_816"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_2_output_0", "onnx::Unsqueeze_816"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_1_output_0"]
Constant_778                     Constant             in=[] out=["onnx::Unsqueeze_818"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Mul_4_output_0", "onnx::Unsqueeze_818"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/Concat_1 Concat               in=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_output_0", "/model/stages/stages.3/blocks/blocks.1/Unsqueeze_1_output_0", "/model/stages/stages.3/blocks/blocks.1/Unsqueeze_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Concat_1_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Reshape Reshape              in=["/model/stages/stages.3/blocks/blocks.1/Concat_output_0", "/model/stages/stages.3/blocks/blocks.1/Concat_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Reshape_output_0"]
/model/stages/stages.3/blocks/blocks.1/Transpose Transpose            in=["/model/stages/stages.3/blocks/blocks.1/Reshape_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Transpose_output_0"] perm=[0, 2, 1]
/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization LayerNormalization   in=["/model/stages/stages.3/blocks/blocks.1/Transpose_output_0", "model.stages.3.blocks.1.norm_xca.weight", "model.stages.3.blocks.1.norm_xca.bias"] out=["/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] weights=model.stages.3.blocks.1.norm_xca.weight[168], model.stages.3.blocks.1.norm_xca.bias[168] axis=-1 epsilon=0.000001
/model/stages/stages.3/blocks/blocks.1/xca/Shape Shape                in=["/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Gather Gather               in=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/Shape_1 Shape                in=["/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_1 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Gather_1 Gather               in=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_1_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_1_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/Shape_2 Shape                in=["/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_2 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Gather_2 Gather               in=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_2_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_2_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/qkv/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.1/norm_xca/LayerNormalization_output_0", "onnx::MatMul_987"] out=["/model/stages/stages.3/blocks/blocks.1/xca/qkv/MatMul_output_0"] weights=onnx::MatMul_987[168, 504]
/model/stages/stages.3/blocks/blocks.1/xca/qkv/Add Add                  in=["model.stages.3.blocks.1.xca.qkv.bias", "/model/stages/stages.3/blocks/blocks.1/xca/qkv/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/qkv/Add_output_0"] weights=model.stages.3.blocks.1.xca.qkv.bias[504]
Constant_795                     Constant             in=[] out=["onnx::Unsqueeze_836"]
/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_output_0", "onnx::Unsqueeze_836"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_output_0"]
Constant_797                     Constant             in=[] out=["onnx::Unsqueeze_838"]
/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_1 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_1_output_0", "onnx::Unsqueeze_838"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_3 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_4 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_4_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_5 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_5_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Concat Concat               in=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_1_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_3_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_4_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_5_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Concat_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/Reshape Reshape              in=["/model/stages/stages.3/blocks/blocks.1/xca/qkv/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Concat_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Reshape_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Transpose Transpose            in=["/model/stages/stages.3/blocks/blocks.1/xca/Reshape_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Transpose_output_0"] perm=[2, 0, 3, 4, 1]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_6 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_6_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Split Split                in=["/model/stages/stages.3/blocks/blocks.1/xca/Transpose_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_6_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Split_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Split_output_1", "/model/stages/stages.3/blocks/blocks.1/xca/Split_output_2"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/Constant_7 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_7_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Squeeze Squeeze              in=["/model/stages/stages.3/blocks/blocks.1/xca/Split_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_7_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_8 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_8_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_1 Squeeze              in=["/model/stages/stages.3/blocks/blocks.1/xca/Split_output_1", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_8_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Constant_9 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_9_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_2 Squeeze              in=["/model/stages/stages.3/blocks/blocks.1/xca/Split_output_2", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_9_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2 ReduceL2             in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2_output_0"] axes=[-1] keepdims=1
/model/stages/stages.3/blocks/blocks.1/xca/Constant_10 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_10_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Clip Clip                 in=["/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_10_output_0", ""] out=["/model/stages/stages.3/blocks/blocks.1/xca/Clip_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Shape_3 Shape                in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_3_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Expand Expand               in=["/model/stages/stages.3/blocks/blocks.1/xca/Clip_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Shape_3_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Expand_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Div Div                  in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Expand_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Div_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2_1 ReduceL2             in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2_1_output_0"] axes=[-1] keepdims=1
/model/stages/stages.3/blocks/blocks.1/xca/Constant_11 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/xca/Constant_11_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Clip_1 Clip                 in=["/model/stages/stages.3/blocks/blocks.1/xca/ReduceL2_1_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Constant_11_output_0", ""] out=["/model/stages/stages.3/blocks/blocks.1/xca/Clip_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Shape_4 Shape                in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Shape_4_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Expand_1 Expand               in=["/model/stages/stages.3/blocks/blocks.1/xca/Clip_1_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Shape_4_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Expand_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Div_1 Div                  in=["/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_1_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Expand_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Div_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Transpose_1 Transpose            in=["/model/stages/stages.3/blocks/blocks.1/xca/Div_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Transpose_1_output_0"] perm=[0, 1, 3, 2]
/model/stages/stages.3/blocks/blocks.1/xca/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.1/xca/Div_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Transpose_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/MatMul_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Mul Mul                  in=["/model/stages/stages.3/blocks/blocks.1/xca/MatMul_output_0", "model.stages.3.blocks.1.xca.temperature"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Mul_output_0"] weights=model.stages.3.blocks.1.xca.temperature[4, 1, 1]
/model/stages/stages.3/blocks/blocks.1/xca/Softmax Softmax              in=["/model/stages/stages.3/blocks/blocks.1/xca/Mul_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Softmax_output_0"] axis=-1
/model/stages/stages.3/blocks/blocks.1/xca/MatMul_1 MatMul               in=["/model/stages/stages.3/blocks/blocks.1/xca/Softmax_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Squeeze_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/MatMul_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Transpose_2 Transpose            in=["/model/stages/stages.3/blocks/blocks.1/xca/MatMul_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Transpose_2_output_0"] perm=[0, 3, 1, 2]
Constant_831                     Constant             in=[] out=["onnx::Unsqueeze_879"]
/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_2 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_output_0", "onnx::Unsqueeze_879"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_2_output_0"]
Constant_833                     Constant             in=[] out=["onnx::Unsqueeze_881"]
/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_1_output_0", "onnx::Unsqueeze_881"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_3_output_0"]
Constant_835                     Constant             in=[] out=["onnx::Unsqueeze_883"]
/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/xca/Gather_2_output_0", "onnx::Unsqueeze_883"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_4_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/Concat_1 Concat               in=["/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_2_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_3_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Unsqueeze_4_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Concat_1_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/xca/Reshape_1 Reshape              in=["/model/stages/stages.3/blocks/blocks.1/xca/Transpose_2_output_0", "/model/stages/stages.3/blocks/blocks.1/xca/Concat_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/Reshape_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/xca/proj/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.1/xca/Reshape_1_output_0", "onnx::MatMul_993"] out=["/model/stages/stages.3/blocks/blocks.1/xca/proj/MatMul_output_0"] weights=onnx::MatMul_993[168, 168]
/model/stages/stages.3/blocks/blocks.1/xca/proj/Add Add                  in=["model.stages.3.blocks.1.xca.proj.bias", "/model/stages/stages.3/blocks/blocks.1/xca/proj/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/xca/proj/Add_output_0"] weights=model.stages.3.blocks.1.xca.proj.bias[168]
/model/stages/stages.3/blocks/blocks.1/Mul_5 Mul                  in=["model.stages.3.blocks.1.gamma_xca", "/model/stages/stages.3/blocks/blocks.1/xca/proj/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_5_output_0"] weights=model.stages.3.blocks.1.gamma_xca[168]
/model/stages/stages.3/blocks/blocks.1/Add_3 Add                  in=["/model/stages/stages.3/blocks/blocks.1/Transpose_output_0", "/model/stages/stages.3/blocks/blocks.1/Mul_5_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Add_3_output_0"]
Constant_843                     Constant             in=[] out=["onnx::Unsqueeze_892"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_3 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_1_output_0", "onnx::Unsqueeze_892"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_3_output_0"]
Constant_845                     Constant             in=[] out=["onnx::Unsqueeze_894"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_4 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_3_output_0", "onnx::Unsqueeze_894"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_4_output_0"]
Constant_847                     Constant             in=[] out=["onnx::Unsqueeze_896"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_5 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_4_output_0", "onnx::Unsqueeze_896"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_5_output_0"]
Constant_849                     Constant             in=[] out=["onnx::Unsqueeze_898"]
/model/stages/stages.3/blocks/blocks.1/Unsqueeze_6 Unsqueeze            in=["/model/stages/stages.3/blocks/blocks.1/Gather_2_output_0", "onnx::Unsqueeze_898"] out=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_6_output_0"]
/model/stages/stages.3/blocks/blocks.1/Concat_2 Concat               in=["/model/stages/stages.3/blocks/blocks.1/Unsqueeze_3_output_0", "/model/stages/stages.3/blocks/blocks.1/Unsqueeze_4_output_0", "/model/stages/stages.3/blocks/blocks.1/Unsqueeze_5_output_0", "/model/stages/stages.3/blocks/blocks.1/Unsqueeze_6_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Concat_2_output_0"] axis=0
/model/stages/stages.3/blocks/blocks.1/Reshape_1 Reshape              in=["/model/stages/stages.3/blocks/blocks.1/Add_3_output_0", "/model/stages/stages.3/blocks/blocks.1/Concat_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Reshape_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/norm/LayerNormalization LayerNormalization   in=["/model/stages/stages.3/blocks/blocks.1/Reshape_1_output_0", "model.stages.3.blocks.1.norm.weight", "model.stages.3.blocks.1.norm.bias"] out=["/model/stages/stages.3/blocks/blocks.1/norm/LayerNormalization_output_0"] weights=model.stages.3.blocks.1.norm.weight[168], model.stages.3.blocks.1.norm.bias[168] axis=-1 epsilon=0.000001
/model/stages/stages.3/blocks/blocks.1/mlp/fc1/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.1/norm/LayerNormalization_output_0", "onnx::MatMul_994"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/fc1/MatMul_output_0"] weights=onnx::MatMul_994[168, 672]
/model/stages/stages.3/blocks/blocks.1/mlp/fc1/Add Add                  in=["model.stages.3.blocks.1.mlp.fc1.bias", "/model/stages/stages.3/blocks/blocks.1/mlp/fc1/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/fc1/Add_output_0"] weights=model.stages.3.blocks.1.mlp.fc1.bias[672]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Div Div                  in=["/model/stages/stages.3/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Div_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Erf Erf                  in=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Div_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Erf_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_1 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Add Add                  in=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Erf_output_0", "/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Add_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul Mul                  in=["/model/stages/stages.3/blocks/blocks.1/mlp/fc1/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/mlp/act/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_2 Constant             in=[] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_2_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul_1 Mul                  in=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul_output_0", "/model/stages/stages.3/blocks/blocks.1/mlp/act/Constant_2_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul_1_output_0"]
/model/stages/stages.3/blocks/blocks.1/mlp/fc2/MatMul MatMul               in=["/model/stages/stages.3/blocks/blocks.1/mlp/act/Mul_1_output_0", "onnx::MatMul_995"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/fc2/MatMul_output_0"] weights=onnx::MatMul_995[672, 168]
/model/stages/stages.3/blocks/blocks.1/mlp/fc2/Add Add                  in=["model.stages.3.blocks.1.mlp.fc2.bias", "/model/stages/stages.3/blocks/blocks.1/mlp/fc2/MatMul_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/mlp/fc2/Add_output_0"] weights=model.stages.3.blocks.1.mlp.fc2.bias[168]
/model/stages/stages.3/blocks/blocks.1/Mul_6 Mul                  in=["model.stages.3.blocks.1.gamma", "/model/stages/stages.3/blocks/blocks.1/mlp/fc2/Add_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Mul_6_output_0"] weights=model.stages.3.blocks.1.gamma[168]
/model/stages/stages.3/blocks/blocks.1/Transpose_1 Transpose            in=["/model/stages/stages.3/blocks/blocks.1/Mul_6_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/stages/stages.3/blocks/blocks.1/Add_4 Add                  in=["/model/stages/stages.3/blocks/blocks.0/Add_output_0", "/model/stages/stages.3/blocks/blocks.1/Transpose_1_output_0"] out=["/model/stages/stages.3/blocks/blocks.1/Add_4_output_0"]
/model/head/global_pool/pool/GlobalAveragePool GlobalAveragePool    in=["/model/stages/stages.3/blocks/blocks.1/Add_4_output_0"] out=["/model/head/global_pool/pool/GlobalAveragePool_output_0"]
/model/head/norm/Transpose       Transpose            in=["/model/head/global_pool/pool/GlobalAveragePool_output_0"] out=["/model/head/norm/Transpose_output_0"] perm=[0, 2, 3, 1]
/model/head/norm/LayerNormalization LayerNormalization   in=["/model/head/norm/Transpose_output_0", "model.head.norm.weight", "model.head.norm.bias"] out=["/model/head/norm/LayerNormalization_output_0"] weights=model.head.norm.weight[168], model.head.norm.bias[168] axis=-1 epsilon=0.000001
/model/head/norm/Transpose_1     Transpose            in=["/model/head/norm/LayerNormalization_output_0"] out=["/model/head/norm/Transpose_1_output_0"] perm=[0, 3, 1, 2]
/model/head/flatten/Flatten      Flatten              in=["/model/head/norm/Transpose_1_output_0"] out=["/model/head/flatten/Flatten_output_0"] axis=1
/model/head/fc/Gemm              Gemm                 in=["/model/head/flatten/Flatten_output_0", "model.head.fc.weight", "model.head.fc.bias"] out=["embedding"] weights=model.head.fc.weight[512, 168], model.head.fc.bias[512] alpha=1 beta=1 transB=1

## Weights
model.stem.0.weight                              [24, 3, 4, 4] f32 = 1152
model.stem.0.bias                                [24] f32 = 24
model.stem.1.weight                              [24] f32 = 24
model.stem.1.bias                                [24] f32 = 24
model.stages.0.blocks.0.gamma                    [24] f32 = 24
model.stages.0.blocks.0.conv_dw.weight           [24, 1, 3, 3] f32 = 216
model.stages.0.blocks.0.conv_dw.bias             [24] f32 = 24
model.stages.0.blocks.0.norm.weight              [24] f32 = 24
model.stages.0.blocks.0.norm.bias                [24] f32 = 24
model.stages.0.blocks.0.mlp.fc1.bias             [96] f32 = 96
model.stages.0.blocks.0.mlp.fc2.bias             [24] f32 = 24
model.stages.0.blocks.1.gamma                    [24] f32 = 24
model.stages.0.blocks.1.conv_dw.weight           [24, 1, 3, 3] f32 = 216
model.stages.0.blocks.1.conv_dw.bias             [24] f32 = 24
model.stages.0.blocks.1.norm.weight              [24] f32 = 24
model.stages.0.blocks.1.norm.bias                [24] f32 = 24
model.stages.0.blocks.1.mlp.fc1.bias             [96] f32 = 96
model.stages.0.blocks.1.mlp.fc2.bias             [24] f32 = 24
model.stages.1.downsample.0.weight               [24] f32 = 24
model.stages.1.downsample.0.bias                 [24] f32 = 24
model.stages.1.downsample.1.weight               [48, 24, 2, 2] f32 = 4608
model.stages.1.downsample.1.bias                 [48] f32 = 48
model.stages.1.blocks.0.gamma                    [48] f32 = 48
model.stages.1.blocks.0.conv_dw.weight           [48, 1, 5, 5] f32 = 1200
model.stages.1.blocks.0.conv_dw.bias             [48] f32 = 48
model.stages.1.blocks.0.norm.weight              [48] f32 = 48
model.stages.1.blocks.0.norm.bias                [48] f32 = 48
model.stages.1.blocks.0.mlp.fc1.bias             [192] f32 = 192
model.stages.1.blocks.0.mlp.fc2.bias             [48] f32 = 48
model.stages.1.blocks.1.gamma_xca                [48] f32 = 48
model.stages.1.blocks.1.gamma                    [48] f32 = 48
model.stages.1.blocks.1.convs.0.weight           [24, 1, 3, 3] f32 = 216
model.stages.1.blocks.1.convs.0.bias             [24] f32 = 24
model.stages.1.blocks.1.pos_embd.token_projection.weight [48, 64, 1, 1] f32 = 3072
model.stages.1.blocks.1.pos_embd.token_projection.bias [48] f32 = 48
model.stages.1.blocks.1.norm_xca.weight          [48] f32 = 48
model.stages.1.blocks.1.norm_xca.bias            [48] f32 = 48
model.stages.1.blocks.1.xca.temperature          [4, 1, 1] f32 = 4
model.stages.1.blocks.1.xca.qkv.bias             [144] f32 = 144
model.stages.1.blocks.1.xca.proj.bias            [48] f32 = 48
model.stages.1.blocks.1.norm.weight              [48] f32 = 48
model.stages.1.blocks.1.norm.bias                [48] f32 = 48
model.stages.1.blocks.1.mlp.fc1.bias             [192] f32 = 192
model.stages.1.blocks.1.mlp.fc2.bias             [48] f32 = 48
model.stages.2.downsample.0.weight               [48] f32 = 48
model.stages.2.downsample.0.bias                 [48] f32 = 48
model.stages.2.downsample.1.weight               [88, 48, 2, 2] f32 = 16896
model.stages.2.downsample.1.bias                 [88] f32 = 88
model.stages.2.blocks.0.gamma                    [88] f32 = 88
model.stages.2.blocks.0.conv_dw.weight           [88, 1, 7, 7] f32 = 4312
model.stages.2.blocks.0.conv_dw.bias             [88] f32 = 88
model.stages.2.blocks.0.norm.weight              [88] f32 = 88
model.stages.2.blocks.0.norm.bias                [88] f32 = 88
model.stages.2.blocks.0.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.0.mlp.fc2.bias             [88] f32 = 88
model.stages.2.blocks.1.gamma                    [88] f32 = 88
model.stages.2.blocks.1.conv_dw.weight           [88, 1, 7, 7] f32 = 4312
model.stages.2.blocks.1.conv_dw.bias             [88] f32 = 88
model.stages.2.blocks.1.norm.weight              [88] f32 = 88
model.stages.2.blocks.1.norm.bias                [88] f32 = 88
model.stages.2.blocks.1.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.1.mlp.fc2.bias             [88] f32 = 88
model.stages.2.blocks.2.gamma                    [88] f32 = 88
model.stages.2.blocks.2.conv_dw.weight           [88, 1, 7, 7] f32 = 4312
model.stages.2.blocks.2.conv_dw.bias             [88] f32 = 88
model.stages.2.blocks.2.norm.weight              [88] f32 = 88
model.stages.2.blocks.2.norm.bias                [88] f32 = 88
model.stages.2.blocks.2.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.2.mlp.fc2.bias             [88] f32 = 88
model.stages.2.blocks.3.gamma                    [88] f32 = 88
model.stages.2.blocks.3.conv_dw.weight           [88, 1, 7, 7] f32 = 4312
model.stages.2.blocks.3.conv_dw.bias             [88] f32 = 88
model.stages.2.blocks.3.norm.weight              [88] f32 = 88
model.stages.2.blocks.3.norm.bias                [88] f32 = 88
model.stages.2.blocks.3.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.3.mlp.fc2.bias             [88] f32 = 88
model.stages.2.blocks.4.gamma                    [88] f32 = 88
model.stages.2.blocks.4.conv_dw.weight           [88, 1, 7, 7] f32 = 4312
model.stages.2.blocks.4.conv_dw.bias             [88] f32 = 88
model.stages.2.blocks.4.norm.weight              [88] f32 = 88
model.stages.2.blocks.4.norm.bias                [88] f32 = 88
model.stages.2.blocks.4.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.4.mlp.fc2.bias             [88] f32 = 88
model.stages.2.blocks.5.gamma_xca                [88] f32 = 88
model.stages.2.blocks.5.gamma                    [88] f32 = 88
model.stages.2.blocks.5.convs.0.weight           [30, 1, 3, 3] f32 = 270
model.stages.2.blocks.5.convs.0.bias             [30] f32 = 30
model.stages.2.blocks.5.convs.1.weight           [30, 1, 3, 3] f32 = 270
model.stages.2.blocks.5.convs.1.bias             [30] f32 = 30
model.stages.2.blocks.5.norm_xca.weight          [88] f32 = 88
model.stages.2.blocks.5.norm_xca.bias            [88] f32 = 88
model.stages.2.blocks.5.xca.temperature          [4, 1, 1] f32 = 4
model.stages.2.blocks.5.xca.qkv.bias             [264] f32 = 264
model.stages.2.blocks.5.xca.proj.bias            [88] f32 = 88
model.stages.2.blocks.5.norm.weight              [88] f32 = 88
model.stages.2.blocks.5.norm.bias                [88] f32 = 88
model.stages.2.blocks.5.mlp.fc1.bias             [352] f32 = 352
model.stages.2.blocks.5.mlp.fc2.bias             [88] f32 = 88
model.stages.3.downsample.0.weight               [88] f32 = 88
model.stages.3.downsample.0.bias                 [88] f32 = 88
model.stages.3.downsample.1.weight               [168, 88, 2, 2] f32 = 59136
model.stages.3.downsample.1.bias                 [168] f32 = 168
model.stages.3.blocks.0.gamma                    [168] f32 = 168
model.stages.3.blocks.0.conv_dw.weight           [168, 1, 9, 9] f32 = 13608
model.stages.3.blocks.0.conv_dw.bias             [168] f32 = 168
model.stages.3.blocks.0.norm.weight              [168] f32 = 168
model.stages.3.blocks.0.norm.bias                [168] f32 = 168
model.stages.3.blocks.0.mlp.fc1.bias             [672] f32 = 672
model.stages.3.blocks.0.mlp.fc2.bias             [168] f32 = 168
model.stages.3.blocks.1.gamma_xca                [168] f32 = 168
model.stages.3.blocks.1.gamma                    [168] f32 = 168
model.stages.3.blocks.1.convs.0.weight           [42, 1, 3, 3] f32 = 378
model.stages.3.blocks.1.convs.0.bias             [42] f32 = 42
model.stages.3.blocks.1.convs.1.weight           [42, 1, 3, 3] f32 = 378
model.stages.3.blocks.1.convs.1.bias             [42] f32 = 42
model.stages.3.blocks.1.convs.2.weight           [42, 1, 3, 3] f32 = 378
model.stages.3.blocks.1.convs.2.bias             [42] f32 = 42
model.stages.3.blocks.1.norm_xca.weight          [168] f32 = 168
model.stages.3.blocks.1.norm_xca.bias            [168] f32 = 168
model.stages.3.blocks.1.xca.temperature          [4, 1, 1] f32 = 4
model.stages.3.blocks.1.xca.qkv.bias             [504] f32 = 504
model.stages.3.blocks.1.xca.proj.bias            [168] f32 = 168
model.stages.3.blocks.1.norm.weight              [168] f32 = 168
model.stages.3.blocks.1.norm.bias                [168] f32 = 168
model.stages.3.blocks.1.mlp.fc1.bias             [672] f32 = 672
model.stages.3.blocks.1.mlp.fc2.bias             [168] f32 = 168
model.head.norm.weight                           [168] f32 = 168
model.head.norm.bias                             [168] f32 = 168
model.head.fc.weight                             [512, 168] f32 = 86016
model.head.fc.bias                               [512] f32 = 512
onnx::MatMul_926                                 [24, 96] f32 = 2304
onnx::MatMul_927                                 [96, 24] f32 = 2304
onnx::MatMul_928                                 [24, 96] f32 = 2304
onnx::MatMul_929                                 [96, 24] f32 = 2304
onnx::MatMul_930                                 [48, 192] f32 = 9216
onnx::MatMul_931                                 [192, 48] f32 = 9216
onnx::MatMul_957                                 [48, 144] f32 = 6912
onnx::MatMul_963                                 [48, 48] f32 = 2304
onnx::MatMul_964                                 [48, 192] f32 = 9216
onnx::MatMul_965                                 [192, 48] f32 = 9216
onnx::MatMul_966                                 [88, 352] f32 = 30976
onnx::MatMul_967                                 [352, 88] f32 = 30976
onnx::MatMul_968                                 [88, 352] f32 = 30976
onnx::MatMul_969                                 [352, 88] f32 = 30976
onnx::MatMul_970                                 [88, 352] f32 = 30976
onnx::MatMul_971                                 [352, 88] f32 = 30976
onnx::MatMul_972                                 [88, 352] f32 = 30976
onnx::MatMul_973                                 [352, 88] f32 = 30976
onnx::MatMul_974                                 [88, 352] f32 = 30976
onnx::MatMul_975                                 [352, 88] f32 = 30976
onnx::MatMul_976                                 [88, 264] f32 = 23232
onnx::MatMul_982                                 [88, 88] f32 = 7744
onnx::MatMul_983                                 [88, 352] f32 = 30976
onnx::MatMul_984                                 [352, 88] f32 = 30976
onnx::MatMul_985                                 [168, 672] f32 = 112896
onnx::MatMul_986                                 [672, 168] f32 = 112896
onnx::MatMul_987                                 [168, 504] f32 = 84672
onnx::MatMul_993                                 [168, 168] f32 = 28224
onnx::MatMul_994                                 [168, 672] f32 = 112896
onnx::MatMul_995                                 [672, 168] f32 = 112896
total parameters: 1244744 (1.24 MB as int8, 4.98 MB as f32)

## Operator counts
Add                      66
Cast                     4
Clip                     6
Concat                   22
Constant                 194
ConstantOfShape          1
Conv                     20
Cos                      2  <-- folded to a constant by export
CumSum                   2  <-- folded to a constant by export
Div                      25
Erf                      12
Expand                   6
Flatten                  1
Gather                   25
Gemm                     1
GlobalAveragePool        1
LayerNormalization       20
MatMul                   36
Mul                      56
Not                      1  <-- folded to a constant by export
ReduceL2                 6
Reshape                  15
Shape                    33
Sin                      2  <-- folded to a constant by export
Slice                    17
Softmax                  3
Split                    3
Squeeze                  9
Transpose                45
Unsqueeze                47

## tract
input 0: 1,3,112,112,F32
tract typed and optimized the model: 528 nodes
output 0: 1,512,F32

all operators are in the allowed list

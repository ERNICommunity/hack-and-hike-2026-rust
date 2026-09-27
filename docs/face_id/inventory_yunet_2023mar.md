# models/face_detection_yunet_2023mar.onnx
ir_version 6, producer pytorch 1.7
opset  11

## Inputs
input: f32[1,3,640,640]

## Outputs
cls_8: f32[1,6400,1]
cls_16: f32[1,1600,1]
cls_32: f32[1,400,1]
obj_8: f32[1,6400,1]
obj_16: f32[1,1600,1]
obj_32: f32[1,400,1]
bbox_8: f32[1,6400,4]
bbox_16: f32[1,1600,4]
bbox_32: f32[1,400,4]
kps_8: f32[1,6400,10]
kps_16: f32[1,1600,10]
kps_32: f32[1,400,10]

## Nodes (106)
Conv_0                           Conv                 in=["input", "420", "421"] out=["419"] weights=420[16, 3, 3, 3], 421[16] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[2, 2]
Relu_1                           Relu                 in=["419"] out=["184"]
Conv_2                           Conv                 in=["184", "backbone.model0.conv2.conv1.weight", "backbone.model0.conv2.conv1.bias"] out=["185"] weights=backbone.model0.conv2.conv1.weight[16, 16, 1, 1], backbone.model0.conv2.conv1.bias[16] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_3                           Conv                 in=["185", "423", "424"] out=["422"] weights=423[16, 1, 3, 3], 424[16] dilations=[1, 1] group=16 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_4                           Relu                 in=["422"] out=["188"]
MaxPool_5                        MaxPool              in=["188"] out=["189"] kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
Conv_6                           Conv                 in=["189", "backbone.model1.conv1.conv1.weight", "backbone.model1.conv1.conv1.bias"] out=["190"] weights=backbone.model1.conv1.conv1.weight[16, 16, 1, 1], backbone.model1.conv1.conv1.bias[16] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_7                           Conv                 in=["190", "426", "427"] out=["425"] weights=426[16, 1, 3, 3], 427[16] dilations=[1, 1] group=16 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_8                           Relu                 in=["425"] out=["193"]
Conv_9                           Conv                 in=["193", "backbone.model1.conv2.conv1.weight", "backbone.model1.conv2.conv1.bias"] out=["194"] weights=backbone.model1.conv2.conv1.weight[32, 16, 1, 1], backbone.model1.conv2.conv1.bias[32] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_10                          Conv                 in=["194", "429", "430"] out=["428"] weights=429[32, 1, 3, 3], 430[32] dilations=[1, 1] group=32 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_11                          Relu                 in=["428"] out=["197"]
Conv_12                          Conv                 in=["197", "backbone.model2.conv1.conv1.weight", "backbone.model2.conv1.conv1.bias"] out=["198"] weights=backbone.model2.conv1.conv1.weight[32, 32, 1, 1], backbone.model2.conv1.conv1.bias[32] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_13                          Conv                 in=["198", "432", "433"] out=["431"] weights=432[32, 1, 3, 3], 433[32] dilations=[1, 1] group=32 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_14                          Relu                 in=["431"] out=["201"]
Conv_15                          Conv                 in=["201", "backbone.model2.conv2.conv1.weight", "backbone.model2.conv2.conv1.bias"] out=["202"] weights=backbone.model2.conv2.conv1.weight[64, 32, 1, 1], backbone.model2.conv2.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_16                          Conv                 in=["202", "435", "436"] out=["434"] weights=435[64, 1, 3, 3], 436[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_17                          Relu                 in=["434"] out=["205"]
MaxPool_18                       MaxPool              in=["205"] out=["206"] kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
Conv_19                          Conv                 in=["206", "backbone.model3.conv1.conv1.weight", "backbone.model3.conv1.conv1.bias"] out=["207"] weights=backbone.model3.conv1.conv1.weight[64, 64, 1, 1], backbone.model3.conv1.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_20                          Conv                 in=["207", "438", "439"] out=["437"] weights=438[64, 1, 3, 3], 439[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_21                          Relu                 in=["437"] out=["210"]
Conv_22                          Conv                 in=["210", "backbone.model3.conv2.conv1.weight", "backbone.model3.conv2.conv1.bias"] out=["211"] weights=backbone.model3.conv2.conv1.weight[64, 64, 1, 1], backbone.model3.conv2.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_23                          Conv                 in=["211", "441", "442"] out=["440"] weights=441[64, 1, 3, 3], 442[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_24                          Relu                 in=["440"] out=["214"]
MaxPool_25                       MaxPool              in=["214"] out=["215"] kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
Conv_26                          Conv                 in=["215", "backbone.model4.conv1.conv1.weight", "backbone.model4.conv1.conv1.bias"] out=["216"] weights=backbone.model4.conv1.conv1.weight[64, 64, 1, 1], backbone.model4.conv1.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_27                          Conv                 in=["216", "444", "445"] out=["443"] weights=444[64, 1, 3, 3], 445[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_28                          Relu                 in=["443"] out=["219"]
Conv_29                          Conv                 in=["219", "backbone.model4.conv2.conv1.weight", "backbone.model4.conv2.conv1.bias"] out=["220"] weights=backbone.model4.conv2.conv1.weight[64, 64, 1, 1], backbone.model4.conv2.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_30                          Conv                 in=["220", "447", "448"] out=["446"] weights=447[64, 1, 3, 3], 448[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_31                          Relu                 in=["446"] out=["223"]
MaxPool_32                       MaxPool              in=["223"] out=["224"] kernel_shape=[2, 2] pads=[0, 0, 0, 0] strides=[2, 2]
Conv_33                          Conv                 in=["224", "backbone.model5.conv1.conv1.weight", "backbone.model5.conv1.conv1.bias"] out=["225"] weights=backbone.model5.conv1.conv1.weight[64, 64, 1, 1], backbone.model5.conv1.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_34                          Conv                 in=["225", "450", "451"] out=["449"] weights=450[64, 1, 3, 3], 451[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_35                          Relu                 in=["449"] out=["228"]
Conv_36                          Conv                 in=["228", "backbone.model5.conv2.conv1.weight", "backbone.model5.conv2.conv1.bias"] out=["229"] weights=backbone.model5.conv2.conv1.weight[64, 64, 1, 1], backbone.model5.conv2.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_37                          Conv                 in=["229", "453", "454"] out=["452"] weights=453[64, 1, 3, 3], 454[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_38                          Relu                 in=["452"] out=["232"]
Conv_39                          Conv                 in=["232", "neck.lateral_convs.2.conv1.weight", "neck.lateral_convs.2.conv1.bias"] out=["233"] weights=neck.lateral_convs.2.conv1.weight[64, 64, 1, 1], neck.lateral_convs.2.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_40                          Conv                 in=["233", "456", "457"] out=["455"] weights=456[64, 1, 3, 3], 457[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_41                          Relu                 in=["455"] out=["236"]
Resize_43                        Resize               in=["236", "240", "464"] out=["241"] weights=240[0], 464[4] mode=nearest
Add_44                           Add                  in=["223", "241"] out=["242"]
Conv_45                          Conv                 in=["242", "neck.lateral_convs.1.conv1.weight", "neck.lateral_convs.1.conv1.bias"] out=["243"] weights=neck.lateral_convs.1.conv1.weight[64, 64, 1, 1], neck.lateral_convs.1.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_46                          Conv                 in=["243", "459", "460"] out=["458"] weights=459[64, 1, 3, 3], 460[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_47                          Relu                 in=["458"] out=["246"]
Resize_49                        Resize               in=["246", "240", "465"] out=["251"] weights=240[0], 465[4] mode=nearest
Add_50                           Add                  in=["214", "251"] out=["252"]
Conv_51                          Conv                 in=["252", "neck.lateral_convs.0.conv1.weight", "neck.lateral_convs.0.conv1.bias"] out=["253"] weights=neck.lateral_convs.0.conv1.weight[64, 64, 1, 1], neck.lateral_convs.0.conv1.bias[64] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_52                          Conv                 in=["253", "462", "463"] out=["461"] weights=462[64, 1, 3, 3], 463[64] dilations=[1, 1] group=64 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Relu_53                          Relu                 in=["461"] out=["256"]
Conv_54                          Conv                 in=["256", "bbox_head.multi_level_cls.0.conv1.weight", "bbox_head.multi_level_cls.0.conv1.bias"] out=["257"] weights=bbox_head.multi_level_cls.0.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_cls.0.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_55                          Conv                 in=["257", "bbox_head.multi_level_cls.0.conv2.weight", "bbox_head.multi_level_cls.0.conv2.bias"] out=["258"] weights=bbox_head.multi_level_cls.0.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_cls.0.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_56                          Conv                 in=["246", "bbox_head.multi_level_cls.1.conv1.weight", "bbox_head.multi_level_cls.1.conv1.bias"] out=["259"] weights=bbox_head.multi_level_cls.1.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_cls.1.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_57                          Conv                 in=["259", "bbox_head.multi_level_cls.1.conv2.weight", "bbox_head.multi_level_cls.1.conv2.bias"] out=["260"] weights=bbox_head.multi_level_cls.1.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_cls.1.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_58                          Conv                 in=["236", "bbox_head.multi_level_cls.2.conv1.weight", "bbox_head.multi_level_cls.2.conv1.bias"] out=["261"] weights=bbox_head.multi_level_cls.2.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_cls.2.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_59                          Conv                 in=["261", "bbox_head.multi_level_cls.2.conv2.weight", "bbox_head.multi_level_cls.2.conv2.bias"] out=["262"] weights=bbox_head.multi_level_cls.2.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_cls.2.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_60                          Conv                 in=["256", "bbox_head.multi_level_bbox.0.conv1.weight", "bbox_head.multi_level_bbox.0.conv1.bias"] out=["263"] weights=bbox_head.multi_level_bbox.0.conv1.weight[4, 64, 1, 1], bbox_head.multi_level_bbox.0.conv1.bias[4] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_61                          Conv                 in=["263", "bbox_head.multi_level_bbox.0.conv2.weight", "bbox_head.multi_level_bbox.0.conv2.bias"] out=["264"] weights=bbox_head.multi_level_bbox.0.conv2.weight[4, 1, 3, 3], bbox_head.multi_level_bbox.0.conv2.bias[4] dilations=[1, 1] group=4 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_62                          Conv                 in=["246", "bbox_head.multi_level_bbox.1.conv1.weight", "bbox_head.multi_level_bbox.1.conv1.bias"] out=["265"] weights=bbox_head.multi_level_bbox.1.conv1.weight[4, 64, 1, 1], bbox_head.multi_level_bbox.1.conv1.bias[4] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_63                          Conv                 in=["265", "bbox_head.multi_level_bbox.1.conv2.weight", "bbox_head.multi_level_bbox.1.conv2.bias"] out=["266"] weights=bbox_head.multi_level_bbox.1.conv2.weight[4, 1, 3, 3], bbox_head.multi_level_bbox.1.conv2.bias[4] dilations=[1, 1] group=4 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_64                          Conv                 in=["236", "bbox_head.multi_level_bbox.2.conv1.weight", "bbox_head.multi_level_bbox.2.conv1.bias"] out=["267"] weights=bbox_head.multi_level_bbox.2.conv1.weight[4, 64, 1, 1], bbox_head.multi_level_bbox.2.conv1.bias[4] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_65                          Conv                 in=["267", "bbox_head.multi_level_bbox.2.conv2.weight", "bbox_head.multi_level_bbox.2.conv2.bias"] out=["268"] weights=bbox_head.multi_level_bbox.2.conv2.weight[4, 1, 3, 3], bbox_head.multi_level_bbox.2.conv2.bias[4] dilations=[1, 1] group=4 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_66                          Conv                 in=["256", "bbox_head.multi_level_obj.0.conv1.weight", "bbox_head.multi_level_obj.0.conv1.bias"] out=["269"] weights=bbox_head.multi_level_obj.0.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_obj.0.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_67                          Conv                 in=["269", "bbox_head.multi_level_obj.0.conv2.weight", "bbox_head.multi_level_obj.0.conv2.bias"] out=["270"] weights=bbox_head.multi_level_obj.0.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_obj.0.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_68                          Conv                 in=["246", "bbox_head.multi_level_obj.1.conv1.weight", "bbox_head.multi_level_obj.1.conv1.bias"] out=["271"] weights=bbox_head.multi_level_obj.1.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_obj.1.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_69                          Conv                 in=["271", "bbox_head.multi_level_obj.1.conv2.weight", "bbox_head.multi_level_obj.1.conv2.bias"] out=["272"] weights=bbox_head.multi_level_obj.1.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_obj.1.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_70                          Conv                 in=["236", "bbox_head.multi_level_obj.2.conv1.weight", "bbox_head.multi_level_obj.2.conv1.bias"] out=["273"] weights=bbox_head.multi_level_obj.2.conv1.weight[1, 64, 1, 1], bbox_head.multi_level_obj.2.conv1.bias[1] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_71                          Conv                 in=["273", "bbox_head.multi_level_obj.2.conv2.weight", "bbox_head.multi_level_obj.2.conv2.bias"] out=["274"] weights=bbox_head.multi_level_obj.2.conv2.weight[1, 1, 3, 3], bbox_head.multi_level_obj.2.conv2.bias[1] dilations=[1, 1] group=1 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_72                          Conv                 in=["256", "bbox_head.multi_level_kps.0.conv1.weight", "bbox_head.multi_level_kps.0.conv1.bias"] out=["275"] weights=bbox_head.multi_level_kps.0.conv1.weight[10, 64, 1, 1], bbox_head.multi_level_kps.0.conv1.bias[10] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_73                          Conv                 in=["275", "bbox_head.multi_level_kps.0.conv2.weight", "bbox_head.multi_level_kps.0.conv2.bias"] out=["276"] weights=bbox_head.multi_level_kps.0.conv2.weight[10, 1, 3, 3], bbox_head.multi_level_kps.0.conv2.bias[10] dilations=[1, 1] group=10 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_74                          Conv                 in=["246", "bbox_head.multi_level_kps.1.conv1.weight", "bbox_head.multi_level_kps.1.conv1.bias"] out=["277"] weights=bbox_head.multi_level_kps.1.conv1.weight[10, 64, 1, 1], bbox_head.multi_level_kps.1.conv1.bias[10] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_75                          Conv                 in=["277", "bbox_head.multi_level_kps.1.conv2.weight", "bbox_head.multi_level_kps.1.conv2.bias"] out=["278"] weights=bbox_head.multi_level_kps.1.conv2.weight[10, 1, 3, 3], bbox_head.multi_level_kps.1.conv2.bias[10] dilations=[1, 1] group=10 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Conv_76                          Conv                 in=["236", "bbox_head.multi_level_kps.2.conv1.weight", "bbox_head.multi_level_kps.2.conv1.bias"] out=["279"] weights=bbox_head.multi_level_kps.2.conv1.weight[10, 64, 1, 1], bbox_head.multi_level_kps.2.conv1.bias[10] dilations=[1, 1] group=1 kernel_shape=[1, 1] pads=[0, 0, 0, 0] strides=[1, 1]
Conv_77                          Conv                 in=["279", "bbox_head.multi_level_kps.2.conv2.weight", "bbox_head.multi_level_kps.2.conv2.bias"] out=["280"] weights=bbox_head.multi_level_kps.2.conv2.weight[10, 1, 3, 3], bbox_head.multi_level_kps.2.conv2.bias[10] dilations=[1, 1] group=10 kernel_shape=[3, 3] pads=[1, 1, 1, 1] strides=[1, 1]
Transpose_78                     Transpose            in=["258"] out=["281"] perm=[0, 2, 3, 1]
Reshape_84                       Reshape              in=["281", "290"] out=["291"] weights=290[3]
Sigmoid_85                       Sigmoid              in=["291"] out=["cls_8"]
Transpose_86                     Transpose            in=["260"] out=["293"] perm=[0, 2, 3, 1]
Reshape_92                       Reshape              in=["293", "290"] out=["303"] weights=290[3]
Sigmoid_93                       Sigmoid              in=["303"] out=["cls_16"]
Transpose_94                     Transpose            in=["262"] out=["305"] perm=[0, 2, 3, 1]
Reshape_100                      Reshape              in=["305", "290"] out=["315"] weights=290[3]
Sigmoid_101                      Sigmoid              in=["315"] out=["cls_32"]
Transpose_102                    Transpose            in=["270"] out=["317"] perm=[0, 2, 3, 1]
Reshape_108                      Reshape              in=["317", "290"] out=["327"] weights=290[3]
Sigmoid_109                      Sigmoid              in=["327"] out=["obj_8"]
Transpose_110                    Transpose            in=["272"] out=["329"] perm=[0, 2, 3, 1]
Reshape_116                      Reshape              in=["329", "290"] out=["339"] weights=290[3]
Sigmoid_117                      Sigmoid              in=["339"] out=["obj_16"]
Transpose_118                    Transpose            in=["274"] out=["341"] perm=[0, 2, 3, 1]
Reshape_124                      Reshape              in=["341", "290"] out=["351"] weights=290[3]
Sigmoid_125                      Sigmoid              in=["351"] out=["obj_32"]
Transpose_126                    Transpose            in=["264"] out=["353"] perm=[0, 2, 3, 1]
Reshape_132                      Reshape              in=["353", "362"] out=["bbox_8"] weights=362[3]
Transpose_133                    Transpose            in=["266"] out=["364"] perm=[0, 2, 3, 1]
Reshape_139                      Reshape              in=["364", "362"] out=["bbox_16"] weights=362[3]
Transpose_140                    Transpose            in=["268"] out=["375"] perm=[0, 2, 3, 1]
Reshape_146                      Reshape              in=["375", "362"] out=["bbox_32"] weights=362[3]
Transpose_147                    Transpose            in=["276"] out=["386"] perm=[0, 2, 3, 1]
Reshape_153                      Reshape              in=["386", "395"] out=["kps_8"] weights=395[3]
Transpose_154                    Transpose            in=["278"] out=["397"] perm=[0, 2, 3, 1]
Reshape_160                      Reshape              in=["397", "395"] out=["kps_16"] weights=395[3]
Transpose_161                    Transpose            in=["280"] out=["408"] perm=[0, 2, 3, 1]
Reshape_167                      Reshape              in=["408", "395"] out=["kps_32"] weights=395[3]

## Weights
420                                              [16, 3, 3, 3] f32 = 432
421                                              [16] f32 = 16
423                                              [16, 1, 3, 3] f32 = 144
424                                              [16] f32 = 16
426                                              [16, 1, 3, 3] f32 = 144
427                                              [16] f32 = 16
429                                              [32, 1, 3, 3] f32 = 288
430                                              [32] f32 = 32
432                                              [32, 1, 3, 3] f32 = 288
433                                              [32] f32 = 32
435                                              [64, 1, 3, 3] f32 = 576
436                                              [64] f32 = 64
438                                              [64, 1, 3, 3] f32 = 576
439                                              [64] f32 = 64
441                                              [64, 1, 3, 3] f32 = 576
442                                              [64] f32 = 64
444                                              [64, 1, 3, 3] f32 = 576
445                                              [64] f32 = 64
447                                              [64, 1, 3, 3] f32 = 576
448                                              [64] f32 = 64
450                                              [64, 1, 3, 3] f32 = 576
451                                              [64] f32 = 64
453                                              [64, 1, 3, 3] f32 = 576
454                                              [64] f32 = 64
456                                              [64, 1, 3, 3] f32 = 576
457                                              [64] f32 = 64
459                                              [64, 1, 3, 3] f32 = 576
460                                              [64] f32 = 64
462                                              [64, 1, 3, 3] f32 = 576
463                                              [64] f32 = 64
464                                              [4] f32 = 4
465                                              [4] f32 = 4
backbone.model0.conv2.conv1.bias                 [16] f32 = 16
backbone.model0.conv2.conv1.weight               [16, 16, 1, 1] f32 = 256
backbone.model1.conv1.conv1.bias                 [16] f32 = 16
backbone.model1.conv1.conv1.weight               [16, 16, 1, 1] f32 = 256
backbone.model1.conv2.conv1.bias                 [32] f32 = 32
backbone.model1.conv2.conv1.weight               [32, 16, 1, 1] f32 = 512
backbone.model2.conv1.conv1.bias                 [32] f32 = 32
backbone.model2.conv1.conv1.weight               [32, 32, 1, 1] f32 = 1024
backbone.model2.conv2.conv1.bias                 [64] f32 = 64
backbone.model2.conv2.conv1.weight               [64, 32, 1, 1] f32 = 2048
backbone.model3.conv1.conv1.bias                 [64] f32 = 64
backbone.model3.conv1.conv1.weight               [64, 64, 1, 1] f32 = 4096
backbone.model3.conv2.conv1.bias                 [64] f32 = 64
backbone.model3.conv2.conv1.weight               [64, 64, 1, 1] f32 = 4096
backbone.model4.conv1.conv1.bias                 [64] f32 = 64
backbone.model4.conv1.conv1.weight               [64, 64, 1, 1] f32 = 4096
backbone.model4.conv2.conv1.bias                 [64] f32 = 64
backbone.model4.conv2.conv1.weight               [64, 64, 1, 1] f32 = 4096
backbone.model5.conv1.conv1.bias                 [64] f32 = 64
backbone.model5.conv1.conv1.weight               [64, 64, 1, 1] f32 = 4096
backbone.model5.conv2.conv1.bias                 [64] f32 = 64
backbone.model5.conv2.conv1.weight               [64, 64, 1, 1] f32 = 4096
bbox_head.multi_level_bbox.0.conv1.bias          [4] f32 = 4
bbox_head.multi_level_bbox.0.conv1.weight        [4, 64, 1, 1] f32 = 256
bbox_head.multi_level_bbox.0.conv2.bias          [4] f32 = 4
bbox_head.multi_level_bbox.0.conv2.weight        [4, 1, 3, 3] f32 = 36
bbox_head.multi_level_bbox.1.conv1.bias          [4] f32 = 4
bbox_head.multi_level_bbox.1.conv1.weight        [4, 64, 1, 1] f32 = 256
bbox_head.multi_level_bbox.1.conv2.bias          [4] f32 = 4
bbox_head.multi_level_bbox.1.conv2.weight        [4, 1, 3, 3] f32 = 36
bbox_head.multi_level_bbox.2.conv1.bias          [4] f32 = 4
bbox_head.multi_level_bbox.2.conv1.weight        [4, 64, 1, 1] f32 = 256
bbox_head.multi_level_bbox.2.conv2.bias          [4] f32 = 4
bbox_head.multi_level_bbox.2.conv2.weight        [4, 1, 3, 3] f32 = 36
bbox_head.multi_level_cls.0.conv1.bias           [1] f32 = 1
bbox_head.multi_level_cls.0.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_cls.0.conv2.bias           [1] f32 = 1
bbox_head.multi_level_cls.0.conv2.weight         [1, 1, 3, 3] f32 = 9
bbox_head.multi_level_cls.1.conv1.bias           [1] f32 = 1
bbox_head.multi_level_cls.1.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_cls.1.conv2.bias           [1] f32 = 1
bbox_head.multi_level_cls.1.conv2.weight         [1, 1, 3, 3] f32 = 9
bbox_head.multi_level_cls.2.conv1.bias           [1] f32 = 1
bbox_head.multi_level_cls.2.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_cls.2.conv2.bias           [1] f32 = 1
bbox_head.multi_level_cls.2.conv2.weight         [1, 1, 3, 3] f32 = 9
bbox_head.multi_level_kps.0.conv1.bias           [10] f32 = 10
bbox_head.multi_level_kps.0.conv1.weight         [10, 64, 1, 1] f32 = 640
bbox_head.multi_level_kps.0.conv2.bias           [10] f32 = 10
bbox_head.multi_level_kps.0.conv2.weight         [10, 1, 3, 3] f32 = 90
bbox_head.multi_level_kps.1.conv1.bias           [10] f32 = 10
bbox_head.multi_level_kps.1.conv1.weight         [10, 64, 1, 1] f32 = 640
bbox_head.multi_level_kps.1.conv2.bias           [10] f32 = 10
bbox_head.multi_level_kps.1.conv2.weight         [10, 1, 3, 3] f32 = 90
bbox_head.multi_level_kps.2.conv1.bias           [10] f32 = 10
bbox_head.multi_level_kps.2.conv1.weight         [10, 64, 1, 1] f32 = 640
bbox_head.multi_level_kps.2.conv2.bias           [10] f32 = 10
bbox_head.multi_level_kps.2.conv2.weight         [10, 1, 3, 3] f32 = 90
bbox_head.multi_level_obj.0.conv1.bias           [1] f32 = 1
bbox_head.multi_level_obj.0.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_obj.0.conv2.bias           [1] f32 = 1
bbox_head.multi_level_obj.0.conv2.weight         [1, 1, 3, 3] f32 = 9
bbox_head.multi_level_obj.1.conv1.bias           [1] f32 = 1
bbox_head.multi_level_obj.1.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_obj.1.conv2.bias           [1] f32 = 1
bbox_head.multi_level_obj.1.conv2.weight         [1, 1, 3, 3] f32 = 9
bbox_head.multi_level_obj.2.conv1.bias           [1] f32 = 1
bbox_head.multi_level_obj.2.conv1.weight         [1, 64, 1, 1] f32 = 64
bbox_head.multi_level_obj.2.conv2.bias           [1] f32 = 1
bbox_head.multi_level_obj.2.conv2.weight         [1, 1, 3, 3] f32 = 9
neck.lateral_convs.0.conv1.bias                  [64] f32 = 64
neck.lateral_convs.0.conv1.weight                [64, 64, 1, 1] f32 = 4096
neck.lateral_convs.1.conv1.bias                  [64] f32 = 64
neck.lateral_convs.1.conv1.weight                [64, 64, 1, 1] f32 = 4096
neck.lateral_convs.2.conv1.bias                  [64] f32 = 64
neck.lateral_convs.2.conv1.weight                [64, 64, 1, 1] f32 = 4096
240                                              [0] f32 = 0
290                                              [3] i64 = 3
362                                              [3] i64 = 3
395                                              [3] i64 = 3
total parameters: 53121 (0.05 MB as int8, 0.21 MB as f32)

## Operator counts
Add                      2
Conv                     53
MaxPool                  4
Relu                     15
Reshape                  12
Resize                   2
Sigmoid                  6
Transpose                12

## tract
input 0: 1,3,640,640,F32
tract typed and optimized the model: 262 nodes
output 0: 1,6400,1,F32
output 1: 1,1600,1,F32
output 2: 1,400,1,F32
output 3: 1,6400,1,F32
output 4: 1,1600,1,F32
output 5: 1,400,1,F32
output 6: 1,6400,4,F32
output 7: 1,1600,4,F32
output 8: 1,400,4,F32
output 9: 1,6400,10,F32
output 10: 1,1600,10,F32
output 11: 1,400,10,F32

all operators are in the allowed list

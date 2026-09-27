//! Names for tensors and nodes.
//!
//! The ONNX export of PyTorch names nodes by their module path, for example
//! `/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul`, and some
//! weights only by a number (`onnx::MatMul_926`, or `420` in YuNet). The
//! firmware wants short, stable names, so facekit derives them from the
//! module path: `stages.0.blocks.0.mlp.fc1.weight`. The rules:
//!
//! - the leading `/` and the `model` prefix go,
//! - a segment followed by itself plus an index collapses (`stages/stages.0`
//!   becomes `stages.0`),
//! - segments join with `.`,
//! - for a weight, the final operator segment (`MatMul`, `Conv`) goes and
//!   `.weight` or `.bias` is added.

/// The module path of a node, as a list of segments, collapsed as
/// described in the module docs. The operator segment stays at the end.
pub fn segments(node_name: &str) -> Vec<&str> {
    let raw: Vec<&str> = node_name
        .trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut segments = Vec::with_capacity(raw.len());
    for (index, segment) in raw.iter().enumerate() {
        if index == 0 && *segment == "model" {
            continue;
        }
        let repeated = raw.get(index + 1).is_some_and(|next| {
            next.starts_with(segment) && next[segment.len()..].starts_with('.')
        });
        if !repeated {
            segments.push(*segment);
        }
    }
    segments
}

/// The node name with its operator segment, for golden vectors:
/// `stages.0.blocks.0.Add`.
pub fn node(node_name: &str) -> String {
    segments(node_name).join(".")
}

/// The module path without the operator segment: `stages.0.blocks.0.mlp.fc1`.
/// A name without slashes (`Conv_0`) is kept as it is.
pub fn module(node_name: &str) -> String {
    let mut segments = segments(node_name);
    if segments.len() > 1 {
        segments.pop();
    }
    segments.join(".")
}

/// The firmware name of a weight: its PyTorch name without the `model.`
/// prefix, or, for an unnamed weight, the module path of the node that uses
/// it plus `.weight` or `.bias` (input 1 or 2 of the node).
pub fn parameter(initializer: &str, node_name: &str, input_index: usize) -> String {
    let unnamed = initializer.starts_with("onnx::")
        || initializer
            .chars()
            .all(|character| character.is_ascii_digit());
    if unnamed {
        let suffix = if input_index == 2 { "bias" } else { "weight" };
        format!("{}.{suffix}", module(node_name))
    } else {
        initializer
            .strip_prefix("model.")
            .unwrap_or(initializer)
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_repeated_segments() {
        assert_eq!(
            module("/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul"),
            "stages.0.blocks.0.mlp.fc1"
        );
        assert_eq!(
            node("/model/stages/stages.1/blocks/blocks.1/Add_3"),
            "stages.1.blocks.1.Add_3"
        );
        assert_eq!(module("Conv_0"), "Conv_0");
        assert_eq!(node("/model/head/fc/Gemm"), "head.fc.Gemm");
    }

    #[test]
    fn names_parameters() {
        assert_eq!(
            parameter("model.stem.0.weight", "/model/stem/stem.0/Conv", 1),
            "stem.0.weight"
        );
        assert_eq!(
            parameter(
                "onnx::MatMul_926",
                "/model/stages/stages.0/blocks/blocks.0/mlp/fc1/MatMul",
                1
            ),
            "stages.0.blocks.0.mlp.fc1.weight"
        );
        assert_eq!(parameter("421", "Conv_0", 2), "Conv_0.bias");
    }
}

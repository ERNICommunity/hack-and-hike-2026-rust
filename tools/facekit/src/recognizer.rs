//! The integer recognizer with its buffers, as the board runs it.

use hack_and_hike_vision::nn::{
    Shape, edgeface,
    lanes::{GeluTable, GroupPlan, NormPlan},
};

use crate::tensors::Tensors;

/// The integer recognizer with its buffers, as the board runs it.
pub struct Runner {
    /// The GELU table's storage.
    gelu: Vec<i16>,
    /// The `i16` scratch, with room to align it.
    i16s: Vec<i16>,
    /// The `f32` scratch.
    f32s: Vec<f32>,
    /// The model's group plans.
    plans: Vec<GroupPlan>,
    /// The model's LayerNorm plans.
    norm_plans: Vec<NormPlan>,
    /// The model's padded weights, with room to align them.
    weights: Vec<i8>,
    /// The model's constants, with room to align them.
    constants: Vec<i16>,
    /// The model's `i16` weights, with room to align them.
    wide: Vec<i16>,
}

/// The part of `buffer` from its first 16-byte boundary.
pub fn aligned<T>(buffer: &mut [T]) -> &mut [T] {
    let skip = buffer.as_ptr().align_offset(16);
    &mut buffer[skip..]
}

impl Default for Runner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runner {
    /// Allocate the buffers and build the GELU table.
    pub fn new() -> Self {
        let mut gelu = vec![0i16; GeluTable::LEN];
        GeluTable::build(&mut gelu);
        Self {
            gelu,
            i16s: vec![0i16; edgeface::int8::SCRATCH_I16_LEN + 8],
            f32s: vec![0.0f32; edgeface::int8::SCRATCH_F32_LEN],
            plans: vec![GroupPlan::ZERO; edgeface::int8::MODEL_PLANS],
            norm_plans: vec![
                NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]);
                edgeface::int8::MODEL_NORM_PLANS
            ],
            weights: vec![0i8; edgeface::int8::MODEL_WEIGHTS_LEN + 16],
            constants: vec![0i16; edgeface::int8::MODEL_CONSTANTS_LEN + 8],
            wide: vec![0i16; edgeface::int8::MODEL_WIDE_LEN + 8],
        }
    }

    /// The embedding of an `i8` input (`weights` packed with
    /// [`Tensors::pack_for_lanes`]).
    pub fn forward(&mut self, weights: &Tensors, input: &[i8], embedding: &mut [f32]) {
        self.forward_traced(weights, input, embedding, |_, _, _| {});
    }

    /// [`Runner::forward`] with a trace of every block.
    pub fn forward_traced(
        &mut self,
        weights: &Tensors,
        input: &[i8],
        embedding: &mut [f32],
        trace: impl FnMut(&str, Shape, &[f32]),
    ) {
        let gelu = GeluTable::build(&mut self.gelu);
        // The board compiles the model once, when it starts. Here the
        // weights differ from call to call, and a compilation is quick
        // on a computer.
        let model = edgeface::int8::Model::compile(
            weights,
            edgeface::int8::ModelStorage {
                plans: &mut self.plans,
                norm_plans: &mut self.norm_plans,
                weights: aligned(&mut self.weights),
                constants: aligned(&mut self.constants),
                wide: aligned(&mut self.wide),
            },
        );
        let scratch = edgeface::int8::Scratch::new(aligned(&mut self.i16s), &mut self.f32s);
        model.forward_traced(&gelu, input, scratch, embedding, trace);
    }
}

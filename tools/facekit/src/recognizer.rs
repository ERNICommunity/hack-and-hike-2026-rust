//! The integer recognizer with its buffers, as the board runs it.

use hack_and_hike_vision::nn::{
    Shape, edgeface,
    lanes::{GeluTable, GroupPlan, NormPlan},
};

use crate::tensors::Tensors;

/// The buffers of the integer recognizer.
pub struct Runner {
    /// The GELU table's storage.
    gelu: Vec<i16>,
    /// The `i16` scratch, 16-byte aligned.
    i16s: Vec<i16>,
    /// Where the aligned scratch starts.
    skip: usize,
    /// The `f32` scratch.
    f32s: Vec<f32>,
    /// The group plans.
    plans: Vec<GroupPlan>,
    /// The LayerNorm plans.
    norm_plans: Vec<NormPlan>,
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
        let i16s = vec![0i16; edgeface::int8::SCRATCH_I16_LEN + 8];
        let skip = i16s.as_ptr().align_offset(16) / 2;
        Self {
            gelu,
            i16s,
            skip,
            f32s: vec![0.0f32; edgeface::int8::SCRATCH_F32_LEN],
            plans: vec![GroupPlan::ZERO; edgeface::int8::SCRATCH_PLANS],
            norm_plans: vec![
                NormPlan::new(&[1.0; 8], &[0.0; 8], &[1.0; 8]);
                edgeface::int8::SCRATCH_NORM_PLANS
            ],
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
        let scratch = edgeface::int8::Scratch::new(
            &mut self.i16s[self.skip..],
            &mut self.f32s,
            &mut self.plans,
            &mut self.norm_plans,
        );
        edgeface::int8::forward_traced(weights, &gelu, input, scratch, embedding, trace);
    }
}

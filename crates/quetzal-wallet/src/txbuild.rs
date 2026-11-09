use crate::fees::FeeParams;
use quetzal_primitives::Coin;

pub struct TxBuilder {
    pub fee_params: FeeParams,
}

impl TxBuilder {
    pub fn new(fee_params: FeeParams) -> Self {
        Self { fee_params }
    }

    /// Estimate the fee for a transaction with `n_inputs` inputs and `n_outputs` outputs.
    ///
    /// Approximation: 200 bytes base + 70 bytes per input + 65 bytes per output.
    pub fn estimate_fee(&self, n_inputs: usize, n_outputs: usize) -> Coin {
        let size = 200 + 70 * n_inputs + 65 * n_outputs;
        self.fee_params.estimate_fee(size)
    }
}

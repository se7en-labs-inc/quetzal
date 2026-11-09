//! Exact single-era slot boundaries in picoseconds.
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct SlotBoundaryRequest {
    pub slot_length_picos: u64,
    pub elapsed_picos: u64,
    pub epoch_length: u32,
}
#[derive(Debug, Serialize)]
pub struct SlotBoundary {
    pub floor_slot: u64,
    pub ceiling_slot: u64,
    pub floor_time_picos: u64,
}
impl SlotBoundaryRequest {
    pub fn boundary(&self) -> Option<SlotBoundary> {
        if self.slot_length_picos == 0
            || self.slot_length_picos > 1_000_000_000_000_000
            || self.epoch_length == 0
        {
            return None;
        }
        let floor_slot = self.elapsed_picos / self.slot_length_picos;
        if floor_slot == u64::MAX {
            return None;
        }
        let ceiling_slot =
            floor_slot + u64::from(!self.elapsed_picos.is_multiple_of(self.slot_length_picos));
        Some(SlotBoundary {
            floor_slot,
            ceiling_slot,
            floor_time_picos: floor_slot * self.slot_length_picos,
        })
    }
}

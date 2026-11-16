//! Exact single-era slot boundaries in picoseconds.
use serde::{Deserialize, Serialize};

/// Canonical UTC timestamps use the shortest exact decimal fraction.
pub fn compact_utc_fraction(time: String) -> String {
    if let Some((seconds, fraction)) = time.strip_suffix('Z').and_then(|s| s.split_once('.')) {
        let fraction = fraction.trim_end_matches('0');
        if fraction.is_empty() {
            format!("{seconds}Z")
        } else {
            format!("{seconds}.{fraction}Z")
        }
    } else {
        time
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_fractions_preserve_precision_and_remove_only_trailing_zeroes() {
        for (fraction, expected) in [("800", ".8"), ("010", ".01"), ("001", ".001"), ("000", "")] {
            assert_eq!(
                super::compact_utc_fraction(format!("2026-11-16T19:42:16.{fraction}Z")),
                format!("2026-11-16T19:42:16{expected}Z")
            );
        }
        assert_eq!(
            super::compact_utc_fraction("2026-11-16T19:42:16Z".into()),
            "2026-11-16T19:42:16Z"
        );
    }
}

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

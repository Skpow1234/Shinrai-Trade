//! Deterministic digest of consumer state.

use crate::consumer::{FeedStatus, MdConsumerState};

fn mix(h: &mut u64, byte: u8) {
    *h ^= u64::from(byte);
    *h = h.wrapping_mul(0x0100_0000_01b3);
}

fn mix_u64(h: &mut u64, v: u64) {
    for i in 0..8 {
        mix(h, ((v >> (i * 8)) & 0xff) as u8);
    }
}

fn mix_i64(h: &mut u64, v: i64) {
    mix_u64(h, v.cast_unsigned());
}

/// FNV-1a 64-bit digest of consumer state (stable cross-run).
#[must_use]
pub fn state_digest(state: &MdConsumerState) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;

    mix_u64(&mut hash, state.applied_count());
    mix_u64(&mut hash, state.duplicate_count());
    mix_u64(&mut hash, state.gap_count());

    for id in state.tracked_instruments() {
        mix_u64(&mut hash, id.get());
        mix_u64(&mut hash, state.expected_seq(id));
        if let Some(px) = state.last_price(id) {
            mix_i64(&mut hash, px.scaled());
        } else {
            mix_u64(&mut hash, 0);
        }
        match state.feed_status(id) {
            FeedStatus::Healthy => mix(&mut hash, 0),
            FeedStatus::Degraded { missing_from } => {
                mix(&mut hash, 1);
                mix_u64(&mut hash, missing_from);
            }
        }
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consumer::MdConsumerState;
    use crate::record::{MdKind, MdRecord};
    use shinrai_instruments::{InstrumentId, PriceTicks};

    #[test]
    fn digest_is_stable() {
        let mut s = MdConsumerState::new();
        let inst = InstrumentId::from_u64(1);
        s.apply(MdRecord::new(
            inst,
            1,
            0,
            MdKind::Trade,
            PriceTicks::from_scaled(100),
        ))
        .expect("a");
        let d1 = state_digest(&s);
        let d2 = state_digest(&s);
        assert_eq!(d1, d2);
    }

    #[test]
    fn first_venue_sequence_need_not_be_one() {
        let mut s = MdConsumerState::new();
        let inst = InstrumentId::from_u64(1);
        let outcome = s
            .apply(MdRecord::new(
                inst,
                50_000,
                0,
                MdKind::Trade,
                PriceTicks::from_scaled(100),
            ))
            .expect("apply");
        assert_eq!(outcome, crate::consumer::ApplyOutcome::Applied);
        assert_eq!(s.expected_seq(inst), 50_001);
        assert!(s.is_synced(inst));
    }
}

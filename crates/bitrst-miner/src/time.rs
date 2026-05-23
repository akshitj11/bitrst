//! Block timestamp validation rules.
//!
//! Bitcoin rejects block headers whose timestamps are not strictly greater than
//! the median time past (MTP) of the prior blocks, and also rejects timestamps
//! too far in the future relative to network-adjusted clock time.

/// Maximum seconds a block timestamp may lead network-adjusted time.
pub const MAX_FUTURE_BLOCK_TIME: u32 = 2 * 60 * 60;

/// Returns true when a block timestamp satisfies Bitcoin's basic time rules.
///
/// The timestamp must be strictly greater than the median time past of prior
/// blocks and must not be more than [`MAX_FUTURE_BLOCK_TIME`] seconds ahead of
/// the node's network-adjusted clock.
pub fn valid_block_time(block_time: u32, median_past_time: u32, network_time: u32) -> bool {
    block_time > median_past_time
        && block_time <= network_time.saturating_add(MAX_FUTURE_BLOCK_TIME)
}

#[cfg(test)]
mod tests {
    use super::{valid_block_time, MAX_FUTURE_BLOCK_TIME};

    #[test]
    fn accepts_timestamp_within_future_drift_limit() {
        let median = 1_000;
        let network = 1_500;

        assert!(valid_block_time(1_001, median, network));
        assert!(valid_block_time(
            network + MAX_FUTURE_BLOCK_TIME,
            median,
            network
        ));
    }

    #[test]
    fn rejects_timestamp_at_or_before_median_past_time() {
        assert!(!valid_block_time(1_000, 1_000, 2_000));
        assert!(!valid_block_time(999, 1_000, 2_000));
    }

    #[test]
    fn rejects_timestamp_too_far_in_the_future() {
        let network = 2_000;
        assert!(!valid_block_time(
            network + MAX_FUTURE_BLOCK_TIME + 1,
            1_000,
            network
        ));
    }
}

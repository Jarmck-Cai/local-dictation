//! Bounded, newest-first timing history. Values are metadata only.

pub const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_RECORDS: usize = 300;

pub fn retained_indices(timestamps: &[u64], now_ms: u64) -> Vec<usize> {
    let mut indices: Vec<usize> = timestamps
        .iter()
        .enumerate()
        .filter_map(|(index, timestamp)| {
            (now_ms.saturating_sub(*timestamp) < RETENTION_MS).then_some(index)
        })
        .collect();
    indices.sort_by_key(|index| std::cmp::Reverse(timestamps[*index]));
    indices.truncate(MAX_RECORDS);
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_boundary_and_clock_rollback_are_safe() {
        let now = RETENTION_MS + 100;
        assert_eq!(retained_indices(&[99, 100, 101, now + 1], now), vec![3, 2]);
    }

    #[test]
    fn newest_records_win_even_when_writers_finish_out_of_order() {
        assert_eq!(retained_indices(&[20, 80, 50], 100), vec![1, 2, 0]);
    }

    #[test]
    fn high_volume_is_bounded_without_losing_newest() {
        let times: Vec<u64> = (0..500).collect();
        let kept = retained_indices(&times, 500);
        assert_eq!(kept.len(), MAX_RECORDS);
        assert_eq!(kept.first(), Some(&499));
        assert_eq!(kept.last(), Some(&200));
    }
}

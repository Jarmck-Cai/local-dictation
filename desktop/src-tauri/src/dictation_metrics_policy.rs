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

pub fn stopped_without_audio_outcome(cancelled: bool) -> &'static str {
    if cancelled {
        "cancelled"
    } else {
        "no_audio"
    }
}

pub fn claim_recording_cancel<T>(recording_active: bool, session: &mut Option<T>) -> Option<T> {
    if recording_active {
        session.take()
    } else {
        None
    }
}

/// Request-to-release time includes microphone readiness and silence. Unknown
/// starts must stay unknown rather than inflate estimated time savings.
pub fn recording_elapsed_ms(
    started: Option<std::time::Instant>,
    stopped: std::time::Instant,
) -> Option<u64> {
    stopped
        .checked_duration_since(started?)
        .map(|elapsed| elapsed.as_millis().min(u64::MAX as u128) as u64)
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

    #[test]
    fn cancellation_during_recording_stop_is_not_counted_as_silence() {
        assert_eq!(stopped_without_audio_outcome(true), "cancelled");
        assert_eq!(stopped_without_audio_outcome(false), "no_audio");
    }

    #[test]
    fn recording_cancel_is_claimed_once_and_idle_cancel_cannot_consume_a_session() {
        let mut session = Some("claude");
        assert_eq!(claim_recording_cancel(false, &mut session), None);
        assert_eq!(session, Some("claude"));
        assert_eq!(claim_recording_cancel(true, &mut session), Some("claude"));
        assert_eq!(
            session, None,
            "a tombstone prevents a simultaneous stop from creating another row"
        );
        assert_eq!(claim_recording_cancel(true, &mut session), None);
        assert_eq!(claim_recording_cancel(false, &mut session), None);
    }

    #[test]
    fn unknown_recording_start_is_not_zero_duration() {
        assert_eq!(recording_elapsed_ms(None, std::time::Instant::now()), None);
    }

    #[test]
    fn recording_time_ends_at_release_and_each_session_has_its_own_start() {
        let first = std::time::Instant::now();
        let first_stop = first + std::time::Duration::from_secs(15);
        let second = first_stop + std::time::Duration::from_secs(30);
        let second_stop = second + std::time::Duration::from_secs(4);
        assert_eq!(recording_elapsed_ms(Some(first), first_stop), Some(15_000));
        assert_eq!(recording_elapsed_ms(Some(second), second_stop), Some(4_000));
    }

    #[test]
    fn invalid_recording_order_stays_unknown() {
        let stop = std::time::Instant::now();
        let start = stop + std::time::Duration::from_secs(1);
        assert_eq!(recording_elapsed_ms(Some(start), stop), None);
    }
}

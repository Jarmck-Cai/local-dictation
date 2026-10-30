#[cfg(test)]
mod unicode_copy_tests {
    use super::*;
    #[test]
    fn copy_preparation_never_changes_original_and_releases_clipboard() {
        reset(false);
        let _lease = CopyLease::prepare("synthetic-transcript").unwrap();
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.sequence, 10);
            assert_eq!(s.owner, 9);
            assert!(s.opened.is_none());
        });
    }
    #[test]
    fn accepted_input_copies_real_body_and_all_privacy_markers_without_a_chord() {
        reset(false);
        let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
        assert_eq!(lease.finish(&|| false), Ok(()));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.body, "synthetic-transcript");
            assert!(!s.delayed);
            assert_eq!(s.injected, 0);
            assert_eq!(s.formats, [50001, 50002, 50003, 13]);
            assert!(s.opened.is_none());
        });
    }
    #[test]
    fn new_owner_or_same_owner_new_sequence_survives_copy_settlement() {
        for change_owner in [true, false] {
            reset(false);
            let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
            new_user_copy(change_owner);
            assert_eq!(lease.finish(&|| false), Ok(()));
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!(s.body, "synthetic-new-user-copy");
                assert_eq!(s.restored, 0);
            });
        }
    }
    #[test]
    fn cancellation_is_sampled_after_opening_copy_settlement_lock() {
        reset(false);
        let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            s.injected = 1;
            s.cancel_on_settle = true;
        });
        assert_eq!(
            lease.finish(&|| SYSTEM.with(|s| s.borrow().cancelled)),
            Ok(())
        );
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert!(s.cancelled);
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.sequence, 10);
        });
    }
    #[test]
    fn copy_publication_failure_restores_snapshot_under_same_lock_without_retry() {
        reset(false);
        let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
        SYSTEM.with(|s| s.borrow_mut().fail_retention = true);
        assert!(lease.finish(&|| false).is_err());
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.restored, 1);
            assert_eq!(s.injected, 0);
            assert!(s.opened.is_none());
        });
    }
    #[test]
    fn new_copy_on_prepare_close_or_final_open_is_never_adopted_or_overwritten() {
        for at_close in [true, false] {
            reset(false);
            if at_close {
                SYSTEM.with(|s| s.borrow_mut().copy_on_readonly_close = true);
            }
            let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
            if !at_close {
                SYSTEM.with(|s| s.borrow_mut().copy_on_copy_open = true);
            }
            assert_eq!(lease.finish(&|| false), Ok(()));
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!(s.body, "synthetic-new-user-copy");
                assert_eq!(s.restored, 0);
            });
        }
    }
    #[test]
    fn failed_privacy_marker_publication_restores_original_payload() {
        reset(false);
        let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
        SYSTEM.with(|s| s.borrow_mut().fail_marker = true);
        assert!(lease.finish(&|| false).is_err());
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.restored, 1);
        });
    }
    #[test]
    fn failed_initial_empty_does_not_destroy_original_by_attempting_a_second_empty() {
        reset(false);
        let mut lease = CopyLease::prepare("synthetic-transcript").unwrap();
        SYSTEM.with(|s| s.borrow_mut().fail_first_empty = true);
        assert!(lease.finish(&|| false).is_err());
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.empty_calls, 1);
            assert_eq!(s.sequence, 10);
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.restored, 0);
        });
    }
}

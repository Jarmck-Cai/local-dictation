// Included by check-win32-publication.py --rdp. No native API is linked.
// The schedules are hypotheses; assertions characterize current behavior.

#[cfg(test)]
mod rdp_tests {
    use super::*;

    fn start(formats_at: u64, request_delay: u64, consume_delay: u64) {
        reset(true);
        SYSTEM.with(|s| {
            let mut s = s.borrow_mut();
            s.rdp = true;
            s.remote_formats_at = Some(formats_at);
            s.remote_request_delay = request_delay;
            s.remote_consume_delay = consume_delay;
        });
    }

    struct RdpKeyboard;
    impl keyboard::Keyboard for RdpKeyboard {
        fn key(&mut self, key: keyboard::Key, direction: keyboard::Direction) -> Result<(), ()> {
            if matches!(key, keyboard::Key::V | keyboard::Key::Insert)
                && direction == keyboard::Direction::Press
            {
                SYSTEM.with(|s| {
                    let mut s = s.borrow_mut();
                    s.injected += 1;
                    if s.remote_formats_ready {
                        s.remote_request_at = Some(s.now + s.remote_request_delay);
                    } else {
                        // Model the remote editor still owning its old text.
                        // A real target might instead have an empty clipboard.
                        s.remote_result = Some("synthetic-remote-original");
                    }
                });
            }
            Ok(())
        }
    }

    fn transaction(copy: bool) -> Result<bool, String> {
        let mut clipboard = WinClipboard::new(copy).unwrap();
        policy::transact_with_retention(
            &mut clipboard,
            "synthetic-transcript",
            || {
                keyboard::inject_if_released(
                    &mut RdpKeyboard,
                    keyboard::Chord::CtrlV,
                    || Ok(false),
                    || SYSTEM.with(|s| s.borrow().cancelled),
                    || thread::sleep(Duration::from_millis(100)),
                )
                .map_err(String::from)
            },
            || SYSTEM.with(|s| s.borrow().cancelled),
            copy,
        )
    }

    #[test]
    fn advertised_format_requests_current_text_once_and_restores_local_snapshot() {
        start(0, 80, 0);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-transcript"));
            assert_eq!((s.injected, s.remote_requests, s.restored), (1, 1, 1));
            assert_eq!(s.body, "synthetic-original");
        });
    }

    #[test]
    fn delayed_local_data_request_is_served_within_the_existing_deadline() {
        start(0, 1200, 0);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-transcript"));
            assert_eq!(s.injected, 1);
            assert!(s.now >= 1200);
        });
    }

    #[test]
    fn paste_before_format_advertisement_can_use_old_remote_clipboard_without_a_receipt() {
        start(40, 80, 0);
        assert_eq!(transaction(false), Ok(false));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-remote-original"));
            assert_eq!((s.injected, s.remote_requests, s.restored), (1, 0, 1));
            assert_eq!(s.body, "synthetic-original");
        });
    }

    #[test]
    fn a_local_read_receipt_can_precede_remote_insertion_and_local_restoration() {
        start(0, 80, 1200);
        assert_eq!(transaction(false), Ok(true));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, None, "a receipt is not remote insertion");
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.injected, 1);
        });
        thread::sleep(Duration::from_millis(1200));
        redirect_tick(HWND(1));
        SYSTEM.with(|s| assert_eq!(s.borrow().remote_result, Some("synthetic-transcript")));
    }

    #[test]
    fn prefetch_before_input_can_deliver_text_without_a_post_input_render_receipt() {
        start(0, 80, 0);
        SYSTEM.with(|s| s.borrow_mut().prefetch_before_begin = true);
        assert_eq!(transaction(false), Ok(false));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-transcript"));
            assert_eq!((s.injected, s.remote_requests, s.restored), (1, 1, 1));
        });
    }

    #[test]
    fn short_nonmutating_clipboard_contention_before_input_currently_aborts_the_chord() {
        start(0, 80, 0);
        SYSTEM.with(|s| s.borrow_mut().busy_before_begin_until = Some(40));
        assert_eq!(
            transaction(false),
            Err("manual_paste_clipboard_busy".into())
        );
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, None);
            assert_eq!((s.injected, s.remote_requests, s.restored), (0, 0, 1));
        });
    }

    #[test]
    fn concurrent_user_copy_wins_while_a_remote_request_is_in_flight() {
        for copy_preference in [false, true] {
            start(0, 800, 0);
            SYSTEM.with(|s| s.borrow_mut().copy_at = Some(300));
            assert_eq!(transaction(copy_preference), Ok(false));
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!(s.body, "synthetic-new-user-copy");
                assert_eq!((s.injected, s.restored), (1, 0));
            });
        }
    }

    #[test]
    fn cancellation_restores_the_snapshot_without_reinjecting_or_retaining_output() {
        for copy_preference in [false, true] {
            start(0, 800, 0);
            SYSTEM.with(|s| s.borrow_mut().cancel_at = Some(300));
            assert_eq!(transaction(copy_preference), Err(policy::CANCELLED.into()));
            SYSTEM.with(|s| {
                let s = s.borrow();
                assert_eq!(s.body, "synthetic-original");
                assert_eq!((s.injected, s.remote_requests, s.restored), (1, 0, 1));
            });
        }
    }

    #[test]
    fn cancellation_after_response_cannot_retract_a_remote_edit_already_in_flight() {
        start(0, 80, 1200);
        SYSTEM.with(|s| s.borrow_mut().cancel_at = Some(110));
        assert_eq!(transaction(false), Err(policy::CANCELLED.into()));
        thread::sleep(Duration::from_millis(1200));
        redirect_tick(HWND(1));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-transcript"));
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.injected, 1);
        });
    }

    #[test]
    fn request_after_the_deadline_can_only_read_the_restored_local_clipboard() {
        start(0, 3200, 0);
        assert_eq!(transaction(false), Ok(false));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.body, "synthetic-original");
            assert_eq!(s.remote_result, None);
            assert_eq!((s.injected, s.restored), (1, 1));
            assert_eq!(s.context, 0, "the temporary owner has been destroyed");
        });
        thread::sleep(Duration::from_millis(300));
        redirect_tick(HWND(1));
        SYSTEM.with(|s| {
            let s = s.borrow();
            assert_eq!(s.remote_result, Some("synthetic-original"));
            assert_eq!(s.injected, 1, "never retry an uncertain chord");
        });
    }

    // Deliberately red until a separately evidenced fix handles these modeled
    // failures. They assert actual editor contents, not the local read flag.
    #[cfg(rdp_delivery_gate)]
    #[test]
    fn delivery_gate_format_advertisement_after_the_chord() {
        start(40, 80, 0);
        let _ = transaction(false);
        SYSTEM.with(|s| assert_eq!(s.borrow().remote_result, Some("synthetic-transcript")));
    }

    #[cfg(rdp_delivery_gate)]
    #[test]
    fn delivery_gate_brief_nonmutating_contention_before_the_chord() {
        start(0, 80, 0);
        SYSTEM.with(|s| s.borrow_mut().busy_before_begin_until = Some(40));
        let _ = transaction(false);
        SYSTEM.with(|s| assert_eq!(s.borrow().remote_result, Some("synthetic-transcript")));
    }
}

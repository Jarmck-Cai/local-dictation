//! Side-effect boundaries shared by the real dispatch/clipboard path and offline tests.
use std::sync::Arc;

/// Public event codes are a static whitelist; internal errors may contain details.
pub fn paste_error_code(error: &str) -> &'static str {
    match error {
        "manual_paste_restore_failed" => "clipboard_restore_failed",
        "automatic_paste_unconfirmed" => "input_unconfirmed",
        _ => "failed",
    }
}

/// The Windows protected transaction pumps its own clipboard-owner messages and
/// can wait for a read. It must not occupy Tauri's UI thread while doing so.
pub fn dispatch_automatic_paste<G, D, P>(
    guard: Arc<G>,
    dispatch_ui: D,
    paste: P,
) -> Result<(), String>
where
    G: Send + Sync + 'static,
    D: FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), String>,
    P: FnOnce() + Send + 'static,
{
    #[cfg(target_os = "windows")]
    {
        let _ = dispatch_ui;
        dispatch_paste(
            guard,
            |work| {
                std::thread::Builder::new()
                    .name("automatic-paste".into())
                    .spawn(work)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
            paste,
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        dispatch_paste(guard, dispatch_ui, paste)
    }
}

pub fn dispatch_paste<G, D, P, E>(guard: Arc<G>, dispatch: D, paste: P) -> Result<(), E>
where
    G: Send + Sync + 'static,
    D: FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), E>,
    P: FnOnce() + Send + 'static,
{
    dispatch(Box::new(move || {
        // The caller retains its own reference until dispatch errors are handled.
        // A queued callback owns this reference until paste and UI cleanup end.
        let _guard = guard;
        paste();
    }))
}

pub fn clipboard_submission<D, C, I, R>(
    delay: D,
    is_cancelled: C,
    inject: I,
    restore: R,
) -> Result<bool, String>
where
    D: FnOnce(),
    C: Fn() -> bool,
    I: FnOnce() -> Result<bool, String>,
    R: FnOnce(),
{
    delay();
    let result = if is_cancelled() { Ok(false) } else { inject() };
    restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[test]
    fn error_events_distinguish_restore_failure_without_exposing_internal_details() {
        assert_eq!(
            paste_error_code("manual_paste_restore_failed"),
            "clipboard_restore_failed"
        );
        assert_eq!(
            paste_error_code("automatic_paste_unconfirmed"),
            "input_unconfirmed"
        );
        for error in [
            "keys_held",
            "manual_paste_backup_failed",
            "private synthetic diagnostic",
            "",
        ] {
            assert_eq!(paste_error_code(error), "failed");
        }
    }

    struct Finish(Arc<AtomicBool>);
    impl Drop for Finish {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn queued_delivery_keeps_task_busy_until_paste_and_ui_finish() {
        let finished = Arc::new(AtomicBool::new(false));
        let guard = Arc::new(Finish(Arc::clone(&finished)));
        let queue = Mutex::new(None);
        let injected = Arc::new(AtomicUsize::new(0));
        let injected_for_paste = Arc::clone(&injected);
        let finished_in_paste = Arc::clone(&finished);
        dispatch_paste(
            Arc::clone(&guard),
            |work| {
                *queue.lock().unwrap() = Some(work);
                Ok::<_, ()>(())
            },
            move || {
                assert!(!finished_in_paste.load(Ordering::SeqCst));
                injected_for_paste.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        drop(guard); // The async transcription body has returned.

        assert!(
            !finished.load(Ordering::SeqCst),
            "queueing is not completion"
        );
        assert_eq!(injected.load(Ordering::SeqCst), 0);
        queue.lock().unwrap().take().unwrap()();
        assert_eq!(injected.load(Ordering::SeqCst), 1);
        assert!(finished.load(Ordering::SeqCst));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn automatic_worker_returns_to_ui_and_keeps_guard_until_transaction_settles() {
        struct Completion(std::sync::mpsc::Sender<()>);
        impl Drop for Completion {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (complete_tx, complete_rx) = std::sync::mpsc::channel();
        let guard = Arc::new(Completion(complete_tx));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (settle_tx, settle_rx) = std::sync::mpsc::channel();
        let ui_thread = std::thread::current().id();
        dispatch_automatic_paste(
            Arc::clone(&guard),
            |_| panic!("Windows protected paste must not run on the UI dispatcher"),
            move || {
                assert_ne!(std::thread::current().id(), ui_thread);
                assert_eq!(std::thread::current().name(), Some("automatic-paste"));
                started_tx.send(()).unwrap();
                settle_rx.recv().unwrap(); // Simulate waiting for read/restoration.
            },
        )
        .unwrap();
        drop(guard);
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            complete_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        settle_tx.send(()).unwrap();
        complete_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn cancellation_during_clipboard_delay_restores_without_injecting() {
        let cancelled = Cell::new(false);
        let injected = Cell::new(0);
        let restored = Cell::new(false);
        let result = clipboard_submission(
            || cancelled.set(true),
            || cancelled.get(),
            || {
                injected.set(injected.get() + 1);
                Ok(true)
            },
            || restored.set(true),
        );
        assert_eq!(result, Ok(false));
        assert_eq!(injected.get(), 0);
        assert!(restored.get());
    }

    #[test]
    fn injection_failure_is_restored_and_never_retried() {
        let attempts = Cell::new(0);
        let restored = Cell::new(false);
        let result = clipboard_submission(
            || {},
            || false,
            || {
                attempts.set(attempts.get() + 1);
                Err("uncertain injection".into())
            },
            || restored.set(true),
        );
        assert_eq!(result, Err("uncertain injection".into()));
        assert_eq!(attempts.get(), 1);
        assert!(restored.get());
    }

    #[test]
    fn rejected_dispatch_does_not_finish_before_caller_handles_error() {
        let finished = Arc::new(AtomicBool::new(false));
        let guard = Arc::new(Finish(Arc::clone(&finished)));
        let result = dispatch_paste(
            Arc::clone(&guard),
            |_| Err("queue closed"),
            || panic!("a rejected dispatcher must not paste"),
        );
        assert_eq!(result, Err("queue closed"));
        assert!(!finished.load(Ordering::SeqCst));
        drop(guard);
        assert!(finished.load(Ordering::SeqCst));
    }
}

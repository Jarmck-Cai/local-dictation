//! Side-effect boundaries shared by the real dispatch/clipboard path and offline tests.
use std::sync::Arc;

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

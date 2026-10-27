//! Ensures playback remains muted through the final capture boundary.
pub fn finish_capture<T>(capture: impl FnOnce() -> T, restore_playback: impl FnOnce()) -> T {
    struct Restore<R: FnOnce()>(Option<R>);
    impl<R: FnOnce()> Drop for Restore<R> {
        fn drop(&mut self) {
            if let Some(restore) = self.0.take() {
                restore();
            }
        }
    }
    let _restore = Restore(Some(restore_playback));
    capture()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn playback_is_muted_through_buffer_and_last_sample_boundary() {
        let muted = Cell::new(true);
        let stages = RefCell::new(Vec::new());
        let samples = finish_capture(
            || {
                stages.borrow_mut().push("buffer");
                assert!(muted.get(), "release buffering still captures audio");
                stages.borrow_mut().push("capture closed");
                assert!(muted.get(), "the last captured block must remain muted");
                vec![1, 2, 3]
            },
            || {
                muted.set(false);
                stages.borrow_mut().push("playback restored");
            },
        );
        stages.borrow_mut().push("stop feedback");
        assert_eq!(samples, vec![1, 2, 3]);
        assert_eq!(
            *stages.borrow(),
            [
                "buffer",
                "capture closed",
                "playback restored",
                "stop feedback"
            ]
        );
        assert!(!muted.get());
    }

    #[test]
    fn empty_or_failed_capture_still_restores_playback() {
        let restored = Cell::new(false);
        let result: Result<Vec<f32>, &str> =
            finish_capture(|| Err("device disconnected"), || restored.set(true));
        assert_eq!(result, Err("device disconnected"));
        assert!(restored.get());
        restored.set(false);
        let result: Option<Vec<f32>> = finish_capture(|| None, || restored.set(true));
        assert_eq!(result, None);
        assert!(restored.get());
    }

    #[test]
    fn unexpected_capture_unwind_still_restores_playback() {
        let restored = Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            finish_capture(|| panic!("capture worker failed"), || restored.set(true));
        }));
        assert!(result.is_err());
        assert!(restored.get());
    }
}

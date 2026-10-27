use std::sync::atomic::{AtomicBool, Ordering};

pub struct PressLatch(AtomicBool);

impl PressLatch {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn update(&self, pressed: bool) -> bool {
        if pressed {
            !self.0.swap(true, Ordering::AcqRel)
        } else {
            self.0.store(false, Ordering::Release);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_repeat_toggles_only_once_and_next_tap_works() {
        let latch = PressLatch::new();
        assert!(latch.update(true));
        assert!(!latch.update(true));
        assert!(!latch.update(true));
        assert!(!latch.update(false));
        assert!(latch.update(true));
    }

    #[test]
    fn release_without_press_never_toggles() {
        let latch = PressLatch::new();
        assert!(!latch.update(false));
        assert!(!latch.update(false));
    }
}

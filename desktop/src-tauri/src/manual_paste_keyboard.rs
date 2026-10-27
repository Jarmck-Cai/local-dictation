//! One manual-paste keyboard attempt; no OS access and no user key recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Control,
    Shift,
    V,
    Insert,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Press,
    Release,
}
#[derive(Clone, Copy)]
pub enum Chord {
    CtrlV,
    CtrlShiftV,
    ShiftInsert,
}
pub trait Keyboard {
    fn key(&mut self, key: Key, direction: Direction) -> Result<(), ()>;
}
pub const CANCELLED: &str = "manual_paste_cancelled";
pub const INJECTION_FAILED: &str = "manual_paste_injection_failed";
pub const KEYS_HELD: &str = "keys_held";
pub const KEYBOARD_UNAVAILABLE: &str = "keyboard_unavailable";
pub const UNSUPPORTED: &str = "unsupported";

/// Tracks synthetic key-down attempts before dispatch, because a failed OS
/// call may have partially succeeded. Drop also settles unwinding paths.
struct PressedKeys<'a, K: Keyboard> {
    keyboard: &'a mut K,
    pending: Vec<Key>,
}
impl<K: Keyboard> PressedKeys<'_, K> {
    fn press(&mut self, key: Key) -> Result<(), ()> {
        self.pending.push(key);
        self.keyboard.key(key, Direction::Press)
    }
    fn release_last(&mut self) -> bool {
        let Some(key) = self.pending.pop() else {
            return true;
        };
        // An error (or panic) releasing one key cannot skip the other keys.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.keyboard.key(key, Direction::Release)
        }))
        .is_ok_and(|result| result.is_ok())
    }
    fn release_all(&mut self) -> bool {
        let mut released = true;
        while !self.pending.is_empty() {
            released = self.release_last() && released;
        }
        released
    }
}
impl<K: Keyboard> Drop for PressedKeys<'_, K> {
    fn drop(&mut self) {
        self.release_all();
    }
}

pub fn send_once(
    keyboard: &mut impl Keyboard,
    chord: Chord,
    hold: impl FnOnce(),
) -> Result<(), &'static str> {
    let keys: &[Key] = match chord {
        Chord::CtrlV => &[Key::Control, Key::V],
        Chord::CtrlShiftV => &[Key::Control, Key::Shift, Key::V],
        Chord::ShiftInsert => &[Key::Shift, Key::Insert],
    };
    let mut pressed = PressedKeys {
        keyboard,
        pending: Vec::with_capacity(3),
    };
    let mut failed = false;
    for key in keys {
        if pressed.press(*key).is_err() {
            failed = true;
            break;
        }
    }
    if !failed {
        // V/Insert goes down exactly once and is released before holding the
        // modifiers. Release errors still settle every remaining modifier.
        failed = !pressed.release_last();
        if !failed {
            hold();
        }
    }
    failed = !pressed.release_all() || failed;
    if failed {
        Err(INJECTION_FAILED)
    } else {
        Ok(())
    }
}

/// Must be called only after acquiring the shared input lock. No earlier
/// keyboard snapshot authorizes this attempt after backup/lock waits.
pub fn inject_if_released(
    keyboard: &mut impl Keyboard,
    chord: Chord,
    query: impl FnOnce() -> Result<bool, &'static str>,
    cancelled: impl Fn() -> bool,
    hold: impl FnOnce(),
) -> Result<(), &'static str> {
    if cancelled() {
        return Err(CANCELLED);
    }
    match query() {
        Ok(true) => return Err(KEYS_HELD),
        Err(UNSUPPORTED) => return Err(UNSUPPORTED),
        Err(_) => return Err(KEYBOARD_UNAVAILABLE),
        Ok(false) => {}
    }
    if cancelled() {
        return Err(CANCELLED);
    }
    send_once(keyboard, chord, hold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[derive(Default)]
    struct Fake {
        commands: Vec<(Key, Direction)>,
        failures: Vec<(Key, Direction)>,
    }
    impl Keyboard for Fake {
        fn key(&mut self, key: Key, direction: Direction) -> Result<(), ()> {
            self.commands.push((key, direction));
            if self.failures.contains(&(key, direction)) {
                Err(())
            } else {
                Ok(())
            }
        }
    }
    use Direction::{Press, Release};
    use Key::{Control, Insert, Shift, V};
    #[test]
    fn ctrl_success_v_failure_still_releases_every_attempted_key() {
        let mut keyboard = Fake {
            failures: vec![(V, Press)],
            ..Fake::default()
        };
        assert_eq!(
            send_once(&mut keyboard, Chord::CtrlV, || panic!(
                "no hold after failure"
            )),
            Err(INJECTION_FAILED)
        );
        assert_eq!(
            keyboard.commands,
            [
                (Control, Press),
                (V, Press),
                (V, Release),
                (Control, Release)
            ]
        );
    }
    #[test]
    fn ctrl_success_shift_failure_releases_both_without_pressing_v() {
        let mut keyboard = Fake {
            failures: vec![(Shift, Press)],
            ..Fake::default()
        };
        assert_eq!(
            send_once(&mut keyboard, Chord::CtrlShiftV, || panic!(
                "no hold after failure"
            )),
            Err(INJECTION_FAILED)
        );
        assert_eq!(
            keyboard.commands,
            [
                (Control, Press),
                (Shift, Press),
                (Shift, Release),
                (Control, Release)
            ]
        );
    }
    #[test]
    fn release_failure_does_not_skip_other_releases_or_retry_v() {
        let mut keyboard = Fake {
            failures: vec![(V, Release), (Shift, Release)],
            ..Fake::default()
        };
        assert_eq!(
            send_once(&mut keyboard, Chord::CtrlShiftV, || {}),
            Err(INJECTION_FAILED)
        );
        assert_eq!(
            keyboard.commands,
            [
                (Control, Press),
                (Shift, Press),
                (V, Press),
                (V, Release),
                (Shift, Release),
                (Control, Release)
            ]
        );
    }
    #[test]
    fn failed_first_press_is_also_released_and_insert_is_sent_once() {
        let mut keyboard = Fake {
            failures: vec![(Control, Press)],
            ..Fake::default()
        };
        assert_eq!(
            send_once(&mut keyboard, Chord::CtrlV, || {}),
            Err(INJECTION_FAILED)
        );
        assert_eq!(keyboard.commands, [(Control, Press), (Control, Release)]);
        let mut keyboard = Fake::default();
        let holds = Cell::new(0);
        assert_eq!(
            send_once(&mut keyboard, Chord::ShiftInsert, || holds
                .set(holds.get() + 1)),
            Ok(())
        );
        assert_eq!(holds.get(), 1);
        assert_eq!(
            keyboard.commands,
            [
                (Shift, Press),
                (Insert, Press),
                (Insert, Release),
                (Shift, Release)
            ]
        );
    }
    #[test]
    fn keys_held_or_unavailable_after_lock_prevent_any_key_attempt() {
        for query in [Ok(true), Err("keyboard_unavailable")] {
            let mut keyboard = Fake::default();
            let want = if query.is_ok() {
                KEYS_HELD
            } else {
                KEYBOARD_UNAVAILABLE
            };
            assert_eq!(
                inject_if_released(
                    &mut keyboard,
                    Chord::CtrlV,
                    || query,
                    || false,
                    || panic!("no hold")
                ),
                Err(want)
            );
            assert!(keyboard.commands.is_empty());
        }
    }
    #[test]
    fn cancellation_during_final_key_query_prevents_any_key_attempt() {
        let cancelled = Cell::new(false);
        let mut keyboard = Fake::default();
        assert_eq!(
            inject_if_released(
                &mut keyboard,
                Chord::CtrlV,
                || {
                    cancelled.set(true);
                    Ok(false)
                },
                || cancelled.get(),
                || {}
            ),
            Err(CANCELLED)
        );
        assert!(keyboard.commands.is_empty());
    }
    #[test]
    fn waiting_for_input_lock_cannot_reuse_an_earlier_all_up_result() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            mpsc, Arc, Mutex,
        };
        let keyboard = Arc::new(Mutex::new(Fake::default()));
        let physical_key_down = Arc::new(AtomicBool::new(false));
        assert!(!physical_key_down.load(Ordering::Acquire)); // initial release check
        let guard = keyboard.lock().unwrap();
        let (ready, rx) = mpsc::channel();
        let keyboard_worker = Arc::clone(&keyboard);
        let physical_worker = Arc::clone(&physical_key_down);
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let mut guard = keyboard_worker.lock().unwrap();
            inject_if_released(
                &mut *guard,
                Chord::CtrlV,
                || Ok(physical_worker.load(Ordering::Acquire)),
                || false,
                || {},
            )
        });
        rx.recv().unwrap();
        physical_key_down.store(true, Ordering::Release);
        drop(guard);
        assert_eq!(worker.join().unwrap(), Err(KEYS_HELD));
        assert!(keyboard.lock().unwrap().commands.is_empty());
    }
}

//! Bounded character submission. OS acceptance is not remote insertion proof.
#[cfg(target_os = "windows")]
#[path = "rdp_input_windows.rs"]
pub(crate) mod platform;

pub(crate) const MAX_UTF16_UNITS: usize = 4096;
pub(crate) const CANCELLED: &str = "manual_paste_cancelled";
pub(crate) const COPY_FAILED: &str = "rdp_input_submitted_copy_failed";
// Compatibility ceiling after the U+20000 field failure. This is not a Win32
// limit: keep higher-plane bodies intact for explicit clipboard recovery.
const MAX_MSTSC_SCALAR: u32 = 0x1ffff;

pub(crate) fn is_system_mstsc(image: &str, system_directory: &str) -> bool {
    image.eq_ignore_ascii_case(&format!(
        "{}\\mstsc.exe",
        system_directory.trim_end_matches('\\')
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UnicodeEvent {
    pub unit: u16,
    pub key_up: bool,
}

pub(crate) trait Runtime {
    /// All allocations, copy preparation and blocking keyboard preparation.
    fn prepare(&mut self, text: &str, events: &[UnicodeEvent]) -> Result<(), &'static str>;
    /// Final focus and prepared keyboard checks, without waiting or mutation.
    fn ready(&self) -> Result<(), &'static str>;
    fn submit(&mut self) -> usize;
    /// Sample cancellation again under the optional copy settlement lock.
    fn finish_copy(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str>;
}

pub(crate) fn events(text: &str) -> Result<Vec<UnicodeEvent>, &'static str> {
    if text.is_empty()
        || text.chars().any(|c| {
            c as u32 > MAX_MSTSC_SCALAR || c.is_control() || (c.is_whitespace() && c != ' ')
        })
    {
        return Err("rdp_input_unsupported_text");
    }
    let units = text.encode_utf16().count();
    if units > MAX_UTF16_UNITS {
        return Err("rdp_input_too_long");
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(units * 2)
        .map_err(|_| "rdp_input_unavailable")?;
    for unit in text.encode_utf16() {
        result.push(UnicodeEvent {
            unit,
            key_up: false,
        });
        result.push(UnicodeEvent { unit, key_up: true });
    }
    Ok(result)
}

pub(crate) fn run(
    runtime: &mut impl Runtime,
    text: &str,
    cancelled: impl Fn() -> bool,
) -> Result<(), &'static str> {
    if cancelled() {
        return Err(CANCELLED);
    }
    let events = events(text)?;
    runtime.prepare(text, &events)?;
    if cancelled() {
        return Err(CANCELLED);
    }
    runtime.ready()?;
    if cancelled() {
        return Err(CANCELLED);
    }
    // A failed call may have inserted a prefix. Never retry or change routes.
    let accepted = runtime.submit();
    if accepted != events.len() {
        return Err(if accepted == 0 {
            "rdp_input_rejected"
        } else {
            "rdp_input_uncertain"
        });
    }
    // Cancellation after acceptance cannot undo input. It still prevents an
    // optional clipboard copy at that separate operation's locked commit point.
    runtime.finish_copy(&cancelled).map_err(|_| COPY_FAILED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn cjk_ascii_and_literal_paths_have_only_matching_unicode_pairs() {
        let got = events("中A\\n").unwrap();
        assert_eq!(
            got,
            vec![
                UnicodeEvent {
                    unit: 0x4e2d,
                    key_up: false
                },
                UnicodeEvent {
                    unit: 0x4e2d,
                    key_up: true
                },
                UnicodeEvent {
                    unit: 0x41,
                    key_up: false
                },
                UnicodeEvent {
                    unit: 0x41,
                    key_up: true
                },
                UnicodeEvent {
                    unit: 0x5c,
                    key_up: false
                },
                UnicodeEvent {
                    unit: 0x5c,
                    key_up: true
                },
                UnicodeEvent {
                    unit: 0x6e,
                    key_up: false
                },
                UnicodeEvent {
                    unit: 0x6e,
                    key_up: true
                },
            ]
        );
    }
    #[test]
    fn supplementary_and_combining_characters_keep_each_utf16_unit_and_release() {
        let got = events("😀e\u{301}").unwrap();
        let literal = [0xd83d, 0xde00, 0x65, 0x301];
        assert_eq!(got.len(), 8);
        for (pair, unit) in got.chunks_exact(2).zip(literal) {
            assert_eq!(
                pair,
                [
                    UnicodeEvent {
                        unit,
                        key_up: false
                    },
                    UnicodeEvent { unit, key_up: true }
                ]
            );
        }
    }
    #[test]
    fn controls_and_line_separators_are_rejected_before_preparation() {
        for text in ["", "x\n", "x\r\n", "x\t", "x\0", "x\u{7f}", "x\u{2028}"] {
            let mut runtime = Fake::default();
            assert_eq!(
                run(&mut runtime, text, || false),
                Err("rdp_input_unsupported_text")
            );
            assert_eq!(
                (runtime.prepares, runtime.submissions, runtime.copies),
                (0, 0, 0)
            );
        }
    }
    #[test]
    fn higher_unicode_planes_refuse_the_whole_body_before_any_side_effect() {
        for text in ["prefix𠀀suffix", "😀𠮷END", "中\u{30000}", "字\u{e0100}"] {
            let mut runtime = Fake::default();
            assert_eq!(
                run(&mut runtime, text, || false),
                Err("rdp_input_unsupported_text")
            );
            assert_eq!(
                (runtime.prepares, runtime.submissions, runtime.copies),
                (0, 0, 0)
            );
        }
    }
    #[test]
    fn bounded_batch_rejects_oversize_text_before_any_effect() {
        assert_eq!(events(&"中".repeat(4096)).unwrap().len(), 8192);
        assert_eq!(events(&"😀".repeat(2048)).unwrap().len(), 8192);
        for text in ["a".repeat(4097), "😀".repeat(2049)] {
            let mut runtime = Fake::default();
            assert_eq!(
                run(&mut runtime, &text, || false),
                Err("rdp_input_too_long")
            );
            assert_eq!(runtime.prepares, 0);
        }
    }
    #[derive(Default)]
    struct Fake {
        prepares: usize,
        submissions: usize,
        copies: usize,
        count: usize,
        prepare_error: Option<&'static str>,
        ready_error: Option<&'static str>,
        accept: Option<usize>,
        copy_error: bool,
        cancelled: Option<std::rc::Rc<Cell<bool>>>,
        cancel_at_prepare: bool,
        cancel_at_submit: bool,
    }
    impl Runtime for Fake {
        fn prepare(&mut self, _: &str, events: &[UnicodeEvent]) -> Result<(), &'static str> {
            self.prepares += 1;
            self.count = events.len();
            if self.cancel_at_prepare {
                self.cancelled.as_ref().unwrap().set(true);
            }
            self.prepare_error.map_or(Ok(()), Err)
        }
        fn ready(&self) -> Result<(), &'static str> {
            self.ready_error.map_or(Ok(()), Err)
        }
        fn submit(&mut self) -> usize {
            self.submissions += 1;
            if self.cancel_at_submit {
                self.cancelled.as_ref().unwrap().set(true);
            }
            self.accept.unwrap_or(self.count)
        }
        fn finish_copy(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), &'static str> {
            if !cancelled() {
                self.copies += 1;
            }
            if self.copy_error {
                Err("synthetic_copy_failure")
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn full_acceptance_submits_one_batch_and_settles_copy_once() {
        let mut runtime = Fake::default();
        assert_eq!(run(&mut runtime, "中😀", || false), Ok(()));
        assert_eq!(
            (runtime.count, runtime.submissions, runtime.copies),
            (6, 1, 1)
        );
    }
    #[test]
    fn zero_and_partial_acceptance_never_copy_retry_or_switch_mechanism() {
        for (accept, code) in [
            (0, "rdp_input_rejected"),
            (1, "rdp_input_uncertain"),
            (3, "rdp_input_uncertain"),
        ] {
            let mut runtime = Fake {
                accept: Some(accept),
                ..Default::default()
            };
            assert_eq!(run(&mut runtime, "中A", || false), Err(code));
            assert_eq!((runtime.submissions, runtime.copies), (1, 0));
        }
    }
    #[test]
    fn preparation_focus_and_keyboard_failures_have_zero_submissions() {
        for code in ["input_target_changed", "keys_held", "keyboard_unavailable"] {
            let mut runtime = Fake {
                ready_error: Some(code),
                ..Default::default()
            };
            assert_eq!(run(&mut runtime, "中", || false), Err(code));
            assert_eq!((runtime.submissions, runtime.copies), (0, 0));
        }
        let mut runtime = Fake {
            prepare_error: Some("synthetic_copy_busy"),
            ..Default::default()
        };
        assert_eq!(
            run(&mut runtime, "中", || false),
            Err("synthetic_copy_busy")
        );
        assert_eq!(runtime.submissions, 0);
    }
    #[test]
    fn cancellation_before_or_during_preparation_never_submits() {
        let mut runtime = Fake::default();
        assert_eq!(run(&mut runtime, "中", || true), Err(CANCELLED));
        assert_eq!(runtime.prepares, 0);
        let cancelled = std::rc::Rc::new(Cell::new(false));
        let mut runtime = Fake {
            cancelled: Some(cancelled.clone()),
            cancel_at_prepare: true,
            ..Default::default()
        };
        assert_eq!(run(&mut runtime, "中", || cancelled.get()), Err(CANCELLED));
        assert_eq!((runtime.submissions, runtime.copies), (0, 0));
    }
    #[test]
    fn cancellation_after_full_submission_preserves_acceptance_and_skips_copy() {
        let cancelled = std::rc::Rc::new(Cell::new(false));
        let mut runtime = Fake {
            cancelled: Some(cancelled.clone()),
            cancel_at_submit: true,
            ..Default::default()
        };
        assert_eq!(run(&mut runtime, "中", || cancelled.get()), Ok(()));
        assert_eq!((runtime.submissions, runtime.copies), (1, 0));
    }
    #[test]
    fn copy_failure_reports_that_input_was_already_submitted() {
        let mut runtime = Fake {
            copy_error: true,
            ..Default::default()
        };
        assert_eq!(run(&mut runtime, "中", || false), Err(COPY_FAILED));
        assert_eq!(runtime.submissions, 1);
    }
}

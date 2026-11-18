//! Small, offline-testable policies shared by recording and text output.

/// A playback endpoint already bound to a specific device, not the current default.
pub trait PlaybackEndpoint {
    fn muted(&self) -> Result<bool, String>;
    fn set_muted(&self, muted: bool) -> Result<(), String>;
}

/// Owns one temporary mute and restores that endpoint's original state.
pub struct MuteLease<E: PlaybackEndpoint> {
    endpoint: E,
    previous_muted: bool,
    restoration_pending: bool,
}

impl<E: PlaybackEndpoint> MuteLease<E> {
    pub fn begin(endpoint: E) -> Result<Self, String> {
        let previous_muted = endpoint.muted()?;
        let lease = Self {
            endpoint,
            previous_muted,
            restoration_pending: true,
        };
        // Construct the lease first: even a partially successful mute that reports
        // an error gets a best-effort restore through Drop.
        lease.endpoint.set_muted(true)?;
        Ok(lease)
    }

    pub fn restore(&mut self) -> Result<(), String> {
        if self.restoration_pending {
            self.endpoint.set_muted(self.previous_muted)?;
            self.restoration_pending = false;
        }
        Ok(())
    }
}

impl<E: PlaybackEndpoint> Drop for MuteLease<E> {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

/// Normalizes dictation output while preserving multi-line paragraph breaks and
/// ordered/unordered list newlines.
///
/// - Unifies `\r\n` and `\r` into `\n`
/// - Collapses horizontal whitespace and control characters within each line to single `' '`
/// - Trims leading/trailing horizontal spaces per line and strips leading/trailing blank lines
/// - Caps consecutive newlines at 2 (`\n\n`, at most one blank line between paragraphs)
/// - Never appends a trailing newline to single-line or multi-line text
pub fn normalize_output(text: &str) -> String {
    let mut body = String::with_capacity(text.len());
    let mut pending_newlines: usize = 0;
    let mut separator = false;
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            if !body.is_empty() {
                pending_newlines = pending_newlines.saturating_add(1);
            }
            separator = false;
        } else if character == '\n' {
            if !body.is_empty() {
                pending_newlines = pending_newlines.saturating_add(1);
            }
            separator = false;
        } else if character.is_control() || character.is_whitespace() {
            if !body.is_empty() && pending_newlines == 0 {
                separator = true;
            }
        } else {
            if pending_newlines > 0 {
                for _ in 0..pending_newlines.min(2) {
                    body.push('\n');
                }
                pending_newlines = 0;
                separator = false;
            } else if separator {
                body.push(' ');
                separator = false;
            }
            body.push(character);
        }
    }
    body
}

/// Chooses a recoverable final body, falling back to the original transcription.
pub fn result_body(raw: &str, processed: Option<&str>) -> String {
    let processed = processed.map(normalize_output).unwrap_or_default();
    if processed.is_empty() {
        normalize_output(raw)
    } else {
        processed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct Endpoint {
        id: &'static str,
        states: Rc<RefCell<HashMap<&'static str, bool>>>,
        fail_restore: Rc<RefCell<bool>>,
    }

    impl PlaybackEndpoint for Endpoint {
        fn muted(&self) -> Result<bool, String> {
            self.states
                .borrow()
                .get(self.id)
                .copied()
                .ok_or("gone".into())
        }
        fn set_muted(&self, muted: bool) -> Result<(), String> {
            if !muted && *self.fail_restore.borrow() {
                return Err("unavailable".into());
            }
            self.states.borrow_mut().insert(self.id, muted);
            Ok(())
        }
    }

    fn endpoint(muted: bool) -> Endpoint {
        Endpoint {
            id: "speakers",
            states: Rc::new(RefCell::new(HashMap::from([
                ("speakers", muted),
                ("headset", false),
            ]))),
            fail_restore: Rc::new(RefCell::new(false)),
        }
    }

    #[test]
    fn temporary_mute_restores_the_captured_endpoint_only() {
        let output = endpoint(false);
        let states = output.states.clone();
        let mut lease = MuteLease::begin(output).unwrap();
        assert_eq!(states.borrow()["speakers"], true);
        // A newly selected default is a different endpoint and must stay untouched.
        states.borrow_mut().insert("headset", true);
        lease.restore().unwrap();
        assert_eq!(states.borrow()["speakers"], false);
        assert_eq!(states.borrow()["headset"], true);
    }

    #[test]
    fn original_mute_is_preserved_when_scope_exits() {
        let output = endpoint(true);
        let states = output.states.clone();
        {
            let _lease = MuteLease::begin(output).unwrap();
            states.borrow_mut().insert("speakers", false);
        }
        assert_eq!(states.borrow()["speakers"], true);
    }

    #[test]
    fn scope_exit_restores_on_failure_or_cancellation() {
        let output = endpoint(false);
        let states = output.states.clone();
        let operation = || -> Result<(), ()> {
            let _lease = MuteLease::begin(output).unwrap();
            assert_eq!(states.borrow()["speakers"], true);
            Err(())
        };
        assert!(operation().is_err());
        assert_eq!(states.borrow()["speakers"], false);
    }

    #[test]
    fn restoration_failure_is_reported_and_can_be_retried() {
        let output = endpoint(false);
        let states = output.states.clone();
        let failure = output.fail_restore.clone();
        let mut lease = MuteLease::begin(output).unwrap();
        *failure.borrow_mut() = true;
        assert!(lease.restore().is_err());
        assert_eq!(states.borrow()["speakers"], true);
        *failure.borrow_mut() = false;
        lease.restore().unwrap();
        assert_eq!(states.borrow()["speakers"], false);
    }

    #[test]
    fn real_controls_collapse_but_literal_paths_and_escapes_survive() {
        assert_eq!(
            normalize_output(
                " \r\n运行\tE:\\Projects\\new\\test.rs\0 --name=foo\u{0085}bar\u{2028}baz\u{2029} "
            ),
            "运行 E:\\Projects\\new\\test.rs --name=foo bar baz"
        );
        assert_eq!(
            normalize_output(r"echo \\n --path C:\\temp\\new"),
            r"echo \\n --path C:\\temp\\new"
        );
    }

    #[test]
    fn empty_processed_result_falls_back_to_recoverable_raw_body() {
        assert_eq!(result_body("原始\r\n正文", Some("\t\n")), "原始\n正文");
        assert_eq!(result_body("原始", Some("整理\t正文")), "整理 正文");
    }

    #[test]
    fn preserves_multiline_lists_and_paragraphs_without_trailing_newlines() {
        assert_eq!(
            normalize_output("  单句没有多余尾部换行 \r\n "),
            "单句没有多余尾部换行"
        );
        assert_eq!(
            normalize_output(
                "\r\n三个任务：\r\n  1. 接入云端网关  \r\n  2. 同步技术词表 \n\n\n  3. 本地懒加载兜底 \r\n\n"
            ),
            "三个任务：\n1. 接入云端网关\n2. 同步技术词表\n\n3. 本地懒加载兜底"
        );
    }
}

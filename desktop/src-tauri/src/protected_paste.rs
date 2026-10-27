//! Synchronous, protected clipboard borrowing for the explicit manual-paste action.
//!
//! `Ok(true)` means a delayed-render read was observed after input began. It
//! cannot prove that the target inserted text. The transaction always settles
//! before returning, so the caller must retain its output permit until then.

#[cfg(target_os = "windows")]
#[path = "protected_paste_windows.rs"]
mod platform;
#[cfg(any(target_os = "windows", test))]
#[path = "protected_paste_policy.rs"]
mod policy;

/// Borrow the clipboard, invoke `inject` once, and settle before returning.
/// This function uses the calling worker thread, including its message pump.
pub(crate) fn run(
    text: &str,
    inject: impl FnOnce() -> Result<(), String>,
    is_cancelled: impl Fn() -> bool,
) -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        platform::run(text, inject, is_cancelled)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (text, inject, is_cancelled);
        Err("manual_paste_unavailable".into())
    }
}

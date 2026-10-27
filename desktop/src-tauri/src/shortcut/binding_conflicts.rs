//! Conservative shortcut collision policy for the optional manual-paste binding.
fn canonical(raw: &str) -> Vec<String> {
    let mut parts: Vec<String> = raw
        .split('+')
        .map(|part| {
            let part = part.trim().to_ascii_lowercase();
            let base = part.trim_end_matches("_left").trim_end_matches("_right");
            match base {
                "ctrl" | "control" => "ctrl".into(),
                "alt" | "option" => "alt".into(),
                "cmd" | "command" | "meta" | "super" | "win" | "windows" => "meta".into(),
                "esc" => "escape".into(),
                _ => base.into(),
            }
        })
        .collect();
    parts.sort();
    parts.dedup();
    parts
}

/// Treat side-specific and generic modifier variants as overlapping. A new
/// optional shortcut must never steal an older, more specific shortcut.
pub fn conflicts<'a>(
    candidate: &str,
    bindings: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> bool {
    let candidate = canonical(candidate);
    bindings
        .into_iter()
        .any(|(id, current)| id != "paste_latest_result" && canonical(current) == candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_binding_wins_despite_modifier_order_case_or_aliases() {
        assert!(conflicts(
            "ctrl+shift+f9",
            [("transcribe", "SHIFT + Control + F9")]
        ));
        assert!(conflicts(
            "ctrl+shift+f9",
            [("transcribe", "ctrl_left+shift_right+f9")]
        ));
        assert!(!conflicts(
            "ctrl+shift+f9",
            [
                ("transcribe", "f8"),
                ("copy_latest_result", "ctrl+shift+f8")
            ]
        ));
    }
    #[test]
    fn manual_binding_does_not_conflict_with_itself() {
        assert!(!conflicts(
            "ctrl+shift+f9",
            [("paste_latest_result", "ctrl+shift+f9")]
        ));
        assert!(conflicts("alt+x", [("cancel", "option+x")]));
    }
}

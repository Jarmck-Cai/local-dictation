//! Add newly introduced shortcut bindings without changing stored user choices.
use std::collections::HashMap;
pub fn merge_missing<T>(stored: &mut HashMap<String, T>, defaults: HashMap<String, T>) -> bool {
    let mut updated = false;
    for (id, binding) in defaults {
        if let std::collections::hash_map::Entry::Vacant(entry) = stored.entry(id) {
            entry.insert(binding);
            updated = true;
        }
    }
    updated
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upgrade_adds_manual_binding_without_changing_existing_custom_keys() {
        let mut stored = HashMap::from([
            ("transcribe".into(), "f13"),
            ("copy_latest_result".into(), "alt+f2"),
        ]);
        let defaults = HashMap::from([
            ("transcribe".into(), "f8"),
            ("copy_latest_result".into(), "ctrl+shift+f8"),
            ("paste_latest_result".into(), "ctrl+shift+f9"),
        ]);
        assert!(merge_missing(&mut stored, defaults));
        assert_eq!(stored["transcribe"], "f13");
        assert_eq!(stored["copy_latest_result"], "alt+f2");
        assert_eq!(stored["paste_latest_result"], "ctrl+shift+f9");
    }
    #[test]
    fn user_manual_binding_survives_repeated_loads() {
        let mut stored = HashMap::from([("paste_latest_result".into(), "alt+f10")]);
        let defaults = HashMap::from([("paste_latest_result".into(), "ctrl+shift+f9")]);
        assert!(!merge_missing(&mut stored, defaults));
        assert_eq!(stored["paste_latest_result"], "alt+f10");
    }
}

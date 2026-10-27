//! Shared binding-change transaction and actual registration identity tracking.
use std::collections::HashMap;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub accelerator: String,
}
pub trait Backend {
    fn register(&mut self, binding: &Binding) -> Result<(), String>;
    fn unregister(&mut self, binding: &Binding) -> Result<(), String>;
}
/// A manual registration failure after committing a valid older binding is
/// reported separately; target failure never commits new settings.
pub struct Eligibility {
    pub target_enabled: bool,
    pub manual_before: bool,
    pub manual_after: bool,
}

pub fn change(
    backend: &mut impl Backend,
    old: &Binding,
    new: &Binding,
    manual: &Binding,
    eligibility: Eligibility,
    commit: impl FnOnce(),
) -> Result<Option<String>, String> {
    let changing_manual = old.id == manual.id;
    if !changing_manual {
        backend.unregister(manual)?;
    }
    if let Err(error) = backend.unregister(old) {
        if !changing_manual && eligibility.manual_before {
            let _ = backend.register(manual);
        }
        return Err(error);
    }
    if eligibility.target_enabled {
        if let Err(error) = backend.register(new) {
            // Withdraw only this binding's recorded identity before restoring
            // its old accelerator. Settings are still unchanged.
            let _ = backend.unregister(new);
            let _ = backend.register(old);
            if !changing_manual && eligibility.manual_before {
                let _ = backend.register(manual);
            }
            return Err(error);
        }
    }
    commit();
    if !changing_manual && eligibility.manual_after {
        Ok(backend.register(manual).err())
    } else {
        Ok(None)
    }
}

pub struct RegisteredBindings<T>(HashMap<String, T>);
impl<T> Default for RegisteredBindings<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}
impl<T> RegisteredBindings<T> {
    pub fn record(&mut self, id: String, identity: T) {
        self.0.insert(id, identity);
    }
    pub fn unregister(
        &mut self,
        id: &str,
        remove: impl FnOnce(&T) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Some(identity) = self.0.get(id) {
            remove(identity)?;
            self.0.remove(id);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn b(id: &str, accelerator: &str) -> Binding {
        Binding {
            id: id.into(),
            accelerator: accelerator.into(),
        }
    }
    struct Fake {
        registered: HashMap<String, String>,
        fail_register: Option<String>,
        fail_unregister: Option<String>,
        log: Vec<String>,
        backend: &'static str,
    }
    impl Fake {
        fn new(backend: &'static str) -> Self {
            Self {
                registered: HashMap::new(),
                fail_register: None,
                fail_unregister: None,
                log: Vec::new(),
                backend,
            }
        }
    }
    impl Backend for Fake {
        fn register(&mut self, binding: &Binding) -> Result<(), String> {
            self.log
                .push(format!("{}:register:{}", self.backend, binding.id));
            if self.fail_register.as_deref() == Some(&binding.accelerator) {
                return Err("register_failed".into());
            }
            if self
                .registered
                .iter()
                .any(|(id, accelerator)| id != &binding.id && accelerator == &binding.accelerator)
            {
                return Err("occupied".into());
            }
            self.registered
                .insert(binding.id.clone(), binding.accelerator.clone());
            Ok(())
        }
        fn unregister(&mut self, binding: &Binding) -> Result<(), String> {
            self.log
                .push(format!("{}:unregister:{}", self.backend, binding.id));
            if self.fail_unregister.as_deref() == Some(&binding.id) {
                return Err("unregister_failed".into());
            }
            self.registered.remove(&binding.id);
            Ok(())
        }
    }
    #[test]
    fn cancel_reset_withdraws_actual_manual_for_both_backends_and_later_can_restore() {
        for name in ["tauri", "handy_keys"] {
            let mut backend = Fake::new(name);
            let manual = b("paste_latest_result", "escape");
            backend.register(&manual).unwrap();
            let committed = Cell::new(false);
            assert_eq!(
                change(
                    &mut backend,
                    &b("cancel", "alt+escape"),
                    &b("cancel", "escape"),
                    &manual,
                    Eligibility {
                        target_enabled: false,
                        manual_before: true,
                        manual_after: false
                    },
                    || committed.set(true)
                ),
                Ok(None)
            );
            assert!(committed.get());
            assert!(!backend.registered.contains_key("paste_latest_result"));
            backend
                .register(&b("cancel", "escape"))
                .expect("next lifecycle can claim Escape");
            backend.unregister(&b("cancel", "escape")).unwrap();
            assert_eq!(
                change(
                    &mut backend,
                    &b("cancel", "escape"),
                    &b("cancel", "alt+escape"),
                    &manual,
                    Eligibility {
                        target_enabled: false,
                        manual_before: false,
                        manual_after: true
                    },
                    || {}
                ),
                Ok(None)
            );
            assert_eq!(
                backend
                    .registered
                    .get("paste_latest_result")
                    .map(String::as_str),
                Some("escape")
            );
        }
    }
    #[test]
    fn older_regular_binding_takes_priority_and_failure_preserves_old_settings() {
        let mut backend = Fake::new("tauri");
        let old = b("transcribe", "f8");
        let manual = b("paste_latest_result", "ctrl+shift+f9");
        backend.register(&old).unwrap();
        backend.register(&manual).unwrap();
        assert_eq!(
            change(
                &mut backend,
                &old,
                &b("transcribe", "ctrl+shift+f9"),
                &manual,
                Eligibility {
                    target_enabled: true,
                    manual_before: true,
                    manual_after: false
                },
                || {}
            ),
            Ok(None)
        );
        assert_eq!(
            backend.registered.get("transcribe").map(String::as_str),
            Some("ctrl+shift+f9")
        );
        assert!(!backend.registered.contains_key("paste_latest_result"));
        backend = Fake::new("handy_keys");
        backend.register(&old).unwrap();
        backend.register(&manual).unwrap();
        backend.fail_register = Some("f10".into());
        let committed = Cell::new(false);
        assert!(change(
            &mut backend,
            &old,
            &b("transcribe", "f10"),
            &manual,
            Eligibility {
                target_enabled: true,
                manual_before: true,
                manual_after: true
            },
            || committed.set(true)
        )
        .is_err());
        assert!(!committed.get());
        assert_eq!(
            backend.registered.get("transcribe").map(String::as_str),
            Some("f8")
        );
        assert_eq!(
            backend
                .registered
                .get("paste_latest_result")
                .map(String::as_str),
            Some("ctrl+shift+f9")
        );
    }
    #[test]
    fn failed_manual_withdrawal_does_not_commit_or_touch_the_old_binding() {
        let mut backend = Fake::new("tauri");
        let old = b("cancel", "alt+escape");
        let manual = b("paste_latest_result", "escape");
        backend.register(&manual).unwrap();
        backend.fail_unregister = Some("paste_latest_result".into());
        let committed = Cell::new(false);
        assert!(change(
            &mut backend,
            &old,
            &b("cancel", "escape"),
            &manual,
            Eligibility {
                target_enabled: false,
                manual_before: true,
                manual_after: false
            },
            || committed.set(true)
        )
        .is_err());
        assert!(!committed.get());
        assert_eq!(
            backend
                .registered
                .get("paste_latest_result")
                .map(String::as_str),
            Some("escape")
        );
    }
    #[test]
    fn suspended_capture_never_restores_manual_before_resume() {
        let mut backend = Fake::new("handy_keys");
        let manual = b("paste_latest_result", "escape");
        assert_eq!(
            change(
                &mut backend,
                &b("cancel", "escape"),
                &b("cancel", "alt+escape"),
                &manual,
                Eligibility {
                    target_enabled: false,
                    manual_before: false,
                    manual_after: false
                },
                || {}
            ),
            Ok(None)
        );
        assert!(!backend.registered.contains_key("paste_latest_result"));
    }
    #[test]
    fn unregister_uses_successful_identity_and_retains_it_when_backend_fails() {
        let mut registrations = RegisteredBindings::default();
        registrations.record("paste_latest_result".into(), "escape");
        let identity = Cell::new("");
        assert!(registrations
            .unregister("paste_latest_result", |actual| {
                identity.set(actual);
                Err("failed".into())
            })
            .is_err());
        assert_eq!(identity.get(), "escape");
        registrations
            .unregister("paste_latest_result", |actual| {
                assert_eq!(*actual, "escape");
                Ok(())
            })
            .unwrap();
        registrations
            .unregister("paste_latest_result", |_| {
                panic!("already removed must not unregister a different owner")
            })
            .unwrap();
    }
}

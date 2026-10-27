//! Windows login command ownership without touching the real registry in tests.

pub fn login_command(exe: &str) -> Result<String, &'static str> {
    if !std::path::Path::new(exe).is_absolute()
        || exe.chars().any(|c| c.is_control() || c == '"')
        || !exe.to_ascii_lowercase().ends_with(".exe")
    {
        return Err("invalid_autostart_path");
    }
    Ok(format!("\"{exe}\" --start-hidden"))
}

pub fn owns_command(command: &str, exe: &str) -> bool {
    let command = command.trim();
    let exact = login_command(exe).ok();
    exact.is_some_and(|expected| command.eq_ignore_ascii_case(&expected))
        || command.eq_ignore_ascii_case(exe)
        || command.eq_ignore_ascii_case(&format!("\"{exe}\""))
}

pub fn startup_approved(value: Option<&[u8]>) -> bool {
    value.is_none_or(|bytes| bytes.len() >= 12 && matches!(bytes[0], 2 | 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_paths_with_spaces_and_unicode() {
        assert_eq!(
            login_command(r"E:\语音 工具\local-dictation.exe").unwrap(),
            "\"E:\\语音 工具\\local-dictation.exe\" --start-hidden"
        );
    }

    #[test]
    fn rejects_relative_and_control_character_paths() {
        for path in [
            "local-dictation.exe",
            "E:\\bad\nname.exe",
            "E:\\bad\"name.exe",
        ] {
            assert!(login_command(path).is_err());
        }
    }

    #[test]
    fn only_this_executable_is_owned() {
        let exe = r"E:\Apps\Local Dictation\local-dictation.exe";
        assert!(owns_command(&login_command(exe).unwrap(), exe));
        assert!(owns_command(&format!("{exe} "), exe)); // legacy plugin entry
        assert!(!owns_command(
            &login_command(r"C:\Apps\local-dictation.exe").unwrap(),
            exe
        ));
        assert!(!owns_command(&format!("\"{exe}\" --cleanup-only"), exe));
        assert!(!owns_command(
            &format!("\"{exe}\" --start-hidden & other.exe"),
            exe
        ));
    }

    #[test]
    fn task_manager_disabled_state_is_respected() {
        assert!(startup_approved(None));
        assert!(startup_approved(Some(&[
            2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
        ])));
        assert!(!startup_approved(Some(&[
            3, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0
        ])));
        assert!(!startup_approved(Some(&[3])));
    }
}

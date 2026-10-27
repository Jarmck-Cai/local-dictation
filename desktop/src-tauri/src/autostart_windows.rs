//! Current-user login entry for this executable; never depends on a shell script.
#[path = "autostart_policy.rs"]
mod policy;

use std::io;
use winreg::{enums::*, RegKey, RegValue};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const NAME: &str = "Local Dictation";

fn unavailable(_: impl std::fmt::Display) -> String {
    "autostart_unavailable".into()
}

fn executable() -> Result<String, String> {
    std::env::current_exe()
        .map_err(unavailable)?
        .into_os_string()
        .into_string()
        .map_err(|_| "invalid_autostart_path".into())
}

fn value(key: &RegKey) -> Result<Option<String>, String> {
    match key.get_value(NAME) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(unavailable(error)),
    }
}

fn approved(hkcu: &RegKey) -> Result<bool, String> {
    match hkcu.open_subkey(APPROVED) {
        Ok(key) => match key.get_raw_value(NAME) {
            Ok(value) => {
                Ok(value.vtype == REG_BINARY && policy::startup_approved(Some(&value.bytes)))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(unavailable(error)),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(unavailable(error)),
    }
}

pub fn is_enabled() -> Result<bool, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = match hkcu.open_subkey(RUN) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(unavailable(error)),
    };
    let exe = executable()?;
    Ok(
        value(&run)?.is_some_and(|command| policy::owns_command(&command, &exe))
            && approved(&hkcu)?,
    )
}

/// Explicit user changes may re-enable a Task Manager-disabled item. Startup
/// synchronization only repairs an owned command and preserves the OS override.
pub fn set_enabled(enabled: bool, explicit: bool) -> Result<(), String> {
    let exe = executable()?;
    let command = policy::login_command(&exe).map_err(str::to_owned)?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = hkcu.create_subkey(RUN).map_err(unavailable)?;
    let existing = value(&run)?;
    if existing
        .as_ref()
        .is_some_and(|old| !policy::owns_command(old, &exe))
    {
        return if enabled {
            Err("autostart_other_installation".into())
        } else {
            Ok(())
        };
    }
    if enabled {
        let approval = if explicit {
            match hkcu.open_subkey_with_flags(APPROVED, KEY_READ | KEY_SET_VALUE) {
                Ok(key) => {
                    let old = match key.get_raw_value(NAME) {
                        Ok(value) => Some(value),
                        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                        Err(error) => return Err(unavailable(error)),
                    };
                    Some((key, old))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(unavailable(error)),
            }
        } else {
            None
        };
        run.set_value(NAME, &command).map_err(unavailable)?;
        if explicit {
            let result: Result<(), String> = (|| {
                if let Some((key, _)) = &approval {
                    key.set_raw_value(
                        NAME,
                        &RegValue {
                            vtype: REG_BINARY,
                            bytes: vec![2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                        },
                    )
                    .map_err(unavailable)?;
                }
                if !is_enabled()? {
                    return Err("autostart_unavailable".into());
                }
                Ok(())
            })();
            if result.is_err() {
                let restored = match &existing {
                    Some(old) => run.set_value(NAME, old),
                    None => run.delete_value(NAME),
                };
                let approval_restored = match &approval {
                    Some((key, Some(old))) => key.set_raw_value(NAME, old),
                    Some((key, None)) => match key.delete_value(NAME) {
                        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                        other => other,
                    },
                    None => Ok(()),
                };
                if restored.is_err() || approval_restored.is_err() {
                    log::warn!("Could not restore the previous login registration after a failure");
                }
            }
            result?;
        }
    } else if existing.is_some() {
        run.delete_value(NAME).map_err(unavailable)?;
    }
    Ok(())
}

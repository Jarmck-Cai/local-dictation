//! Per-account device credentials. Keychain items are independent of the data directory.
use crate::dictation_cleanup::SecretBytes;
use security_framework::passwords::{
    delete_generic_password, generic_password, set_generic_password, PasswordOptions,
};

const SERVICE: &str = "com.localdictation.desktop";
const ITEM_NOT_FOUND: i32 = -25300;

pub(crate) const CLAUDE: &str = "claude-api-key";
pub(crate) const GATEWAY: &str = "gateway-device-token";

pub(crate) fn load(account: &str) -> Result<Option<SecretBytes>, ()> {
    match generic_password(PasswordOptions::new_generic_password(SERVICE, account)) {
        Ok(password) => Ok(Some(SecretBytes(password))),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(None),
        Err(_) => Err(()),
    }
}

pub(crate) fn save(account: &str, secret: &SecretBytes) -> Result<(), ()> {
    set_generic_password(SERVICE, account, &secret.0).map_err(|_| ())
}

pub(crate) fn clear(account: &str) -> Result<(), ()> {
    match delete_generic_password(SERVICE, account) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == ITEM_NOT_FOUND => Ok(()),
        Err(_) => Err(()),
    }
}

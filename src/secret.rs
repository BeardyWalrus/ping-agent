//! Keeps the MQTT password out of plain text in config.json using the Windows
//! Data Protection API (DPAPI). Only the same Windows user can decrypt it.
//! Off Windows (tests) the value is stored as is.

use base64::Engine;

const PREFIX: &str = "dpapi:";

/// Encrypt `plain` for storage. Empty stays empty.
pub fn protect(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let bytes = platform::protect(plain.as_bytes())?;
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// Decrypt a stored value. Values without the prefix (hand-edited) are returned as is.
pub fn reveal(stored: &str) -> Result<String, String> {
    let Some(b64) = stored.strip_prefix(PREFIX) else {
        return Ok(stored.to_string());
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("stored password is not valid base64: {e}"))?;
    let plain = platform::unprotect(&bytes)?;
    String::from_utf8(plain).map_err(|_| "stored password is not valid UTF-8".to_string())
}

#[cfg(windows)]
mod platform {
    use std::ptr;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData as _) };
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err("Windows refused to encrypt the password (CryptProtectData failed)".into());
        }
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(
                "could not decrypt the stored password (was it saved by another Windows user?)"
                    .into(),
            );
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        Ok(data.to_vec())
    }
    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        Ok(data.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_marks_protected_values() {
        let stored = protect("hunter2").unwrap();
        assert!(stored.starts_with("dpapi:"));
        assert_ne!(stored, "hunter2");
        assert_eq!(reveal(&stored).unwrap(), "hunter2");
    }

    #[test]
    fn plain_values_pass_through() {
        assert_eq!(reveal("plain-text").unwrap(), "plain-text");
        assert_eq!(protect("").unwrap(), "");
        assert_eq!(reveal("").unwrap(), "");
        assert!(reveal("dpapi:not base64!!").is_err());
    }
}

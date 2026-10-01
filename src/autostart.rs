//! "Start with Windows" via the per-user Run registry key.

use std::ptr;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

use crate::config::APP_NAME;

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Key(HKEY);

impl Key {
    fn open_run() -> Result<Key, String> {
        let mut hkey: HKEY = ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE | KEY_QUERY_VALUE,
                ptr::null(),
                &mut hkey,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "could not open the Run registry key (error {status})"
            ));
        }
        Ok(Key(hkey))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

/// The command line Windows should run at login: the current exe, quoted.
fn launch_command() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this exe: {e}"))?;
    Ok(format!("\"{}\"", exe.display()))
}

pub fn is_enabled() -> bool {
    let Ok(key) = Key::open_run() else {
        return false;
    };
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            wide(APP_NAME).as_ptr(),
            ptr::null(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    status == ERROR_SUCCESS
}

pub fn set_enabled(enable: bool) -> Result<(), String> {
    let key = Key::open_run()?;
    let name = wide(APP_NAME);
    if enable {
        let value = wide(&launch_command()?);
        let bytes = value.len() * std::mem::size_of::<u16>();
        let status = unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                bytes as u32,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "could not write the Run registry value (error {status})"
            ));
        }
    } else {
        let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "could not remove the Run registry value (error {status})"
            ));
        }
    }
    Ok(())
}

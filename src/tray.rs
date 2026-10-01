//! The notification-area ("system tray") icon, talking to the shell directly so
//! the icon can carry a GUID. A GUID gives the icon a stable identity, which is
//! what lets Windows remember "show this on the taskbar" across upgrades.

use std::mem;

use winapi::shared::guiddef::GUID;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::{HICON, HWND};
use winapi::um::shellapi::{
    Shell_NotifyIconW, NIF_GUID, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIM_SETVERSION, NIN_KEYSELECT, NIN_SELECT, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use winapi::um::winuser::{WM_APP, WM_CONTEXTMENU};

/// Window message the shell sends us for clicks on the icon.
pub const CALLBACK_MESSAGE: UINT = WM_APP + 0x1A7;

const ICON_ID: UINT = 1;

/// Fixed identity for the icon. Windows binds it to the exe's path on first use.
const ICON_GUID: GUID = GUID {
    Data1: 0x9c4f1d2e,
    Data2: 0x6b3a,
    Data3: 0x4e8f,
    Data4: [0xa1, 0x57, 0x3d, 0x0b, 0x7e, 0x2c, 0x9f, 0x61],
};

pub struct TrayIcon {
    hwnd: HWND,
    use_guid: bool,
    added: bool,
}

/// What the user did to the icon, with the screen position to open a menu at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ContextMenu { x: i32, y: i32 },
    Select { x: i32, y: i32 },
}

impl TrayIcon {
    pub fn new(hwnd: HWND) -> TrayIcon {
        TrayIcon {
            hwnd,
            use_guid: true,
            added: false,
        }
    }

    fn base(&self, with_guid: bool) -> NOTIFYICONDATAW {
        let mut d: NOTIFYICONDATAW = unsafe { mem::zeroed() };
        d.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
        d.hWnd = self.hwnd;
        d.uID = ICON_ID;
        if with_guid {
            d.uFlags |= NIF_GUID;
            d.guidItem = ICON_GUID;
        }
        d
    }

    fn fill_add(&self, d: &mut NOTIFYICONDATAW, icon: HICON, tip: &str) {
        d.uFlags |= NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        d.uCallbackMessage = CALLBACK_MESSAGE;
        d.hIcon = icon;
        copy_tip(&mut d.szTip, tip);
    }

    /// Put the icon in the tray. Tries the GUID identity first and falls back to a
    /// plain icon if the shell refuses it (it does when the exe has moved since the
    /// GUID was first registered).
    pub fn add(&mut self, icon: HICON, tip: &str) -> Result<(), String> {
        unsafe {
            // A previous instance that was killed rather than exited may have left a
            // stale entry under our GUID, which would make NIM_ADD fail. Clear it.
            let mut stale = self.base(true);
            Shell_NotifyIconW(NIM_DELETE, &mut stale);

            let mut d = self.base(true);
            self.fill_add(&mut d, icon, tip);
            if Shell_NotifyIconW(NIM_ADD, &mut d) != 0 {
                self.use_guid = true;
            } else {
                let mut d = self.base(false);
                self.fill_add(&mut d, icon, tip);
                if Shell_NotifyIconW(NIM_ADD, &mut d) == 0 {
                    return Err("the shell refused to add the tray icon".to_string());
                }
                self.use_guid = false;
            }
            self.added = true;

            let mut v = self.base(self.use_guid);
            *v.u.uVersion_mut() = NOTIFYICON_VERSION_4;
            Shell_NotifyIconW(NIM_SETVERSION, &mut v);
        }
        Ok(())
    }

    pub fn set_icon(&self, icon: HICON) {
        if !self.added {
            return;
        }
        let mut d = self.base(self.use_guid);
        d.uFlags |= NIF_ICON;
        d.hIcon = icon;
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut d) };
    }

    pub fn set_tip(&self, tip: &str) {
        if !self.added {
            return;
        }
        let mut d = self.base(self.use_guid);
        d.uFlags |= NIF_TIP | NIF_SHOWTIP;
        copy_tip(&mut d.szTip, tip);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut d) };
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        if self.added {
            let mut d = self.base(self.use_guid);
            unsafe { Shell_NotifyIconW(NIM_DELETE, &mut d) };
        }
    }
}

/// Copy `tip` into the fixed UTF-16 buffer, truncating and NUL-terminating.
fn copy_tip(dst: &mut [u16; 128], tip: &str) {
    let mut n = 0;
    for unit in tip.encode_utf16() {
        if n >= dst.len() - 1 {
            break;
        }
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}

/// Decode a `CALLBACK_MESSAGE` (NOTIFYICON_VERSION_4 layout: event in the low
/// word of lParam, cursor position packed in wParam).
pub fn decode(wparam: WPARAM, lparam: LPARAM) -> Option<TrayEvent> {
    let event = (lparam & 0xFFFF) as u32;
    let x = (wparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    match event {
        WM_CONTEXTMENU => Some(TrayEvent::ContextMenu { x, y }),
        NIN_SELECT | NIN_KEYSELECT => Some(TrayEvent::Select { x, y }),
        _ => None,
    }
}

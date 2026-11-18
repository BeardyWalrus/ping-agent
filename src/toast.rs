//! Desk notices as a Windows toast that stays on screen until it's dismissed.
//!
//! The tray icon's own notification (the balloon) goes away by itself after a
//! few seconds, which is easy to miss for "Desk air is stuffy". A toast with
//! the `reminder` scenario stays until the owner presses a button on it.
//!
//! Both buttons are handled by Windows itself, so nothing here has to listen
//! for clicks: **Open** (and a click on the toast) opens the box's page with
//! protocol activation, and **Dismiss** is the system's own dismiss. A newer
//! desk notice replaces the one on screen rather than stacking under it.
//!
//! Showing a toast needs an app identity (AUMID). It's registered for the
//! current user at `HKCU\Software\Classes\AppUserModelId\<AUMID>`, and the
//! installer's Start menu shortcut carries it too.

/// The app identity toasts are shown under.
pub const AUMID: &str = "BeardyWalrus.PingAgent";
/// The name Windows shows at the top of the toast.
pub const DISPLAY_NAME: &str = "PingAgent";
/// Tag and group shared by every desk toast, so a new one replaces the old.
pub const DESK_TAG: &str = "desk";
pub const DESK_GROUP: &str = "PingAgent";

/// Escape text for an XML attribute or element.
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no place for other control characters.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The toast XML for a desk notice: title, text, no sound, and the reminder
/// scenario so it stays until dismissed. With a `link`, a click and the Open
/// button open it; without one, only Dismiss is offered.
pub fn desk_toast_xml(title: &str, text: &str, link: Option<&str>) -> String {
    let title = xml_escape(title);
    let text = xml_escape(text);
    let (launch, open) = match link {
        Some(url) => {
            let url = xml_escape(url);
            (
                format!(r#" activationType="protocol" launch="{url}""#),
                format!(r#"<action content="Open" activationType="protocol" arguments="{url}"/>"#),
            )
        }
        None => (String::new(), String::new()),
    };
    format!(
        concat!(
            r#"<toast scenario="reminder"{launch}>"#,
            r#"<visual><binding template="ToastGeneric">"#,
            r#"<text>{title}</text><text>{text}</text>"#,
            r#"</binding></visual>"#,
            r#"<audio silent="true"/>"#,
            r#"<actions>{open}"#,
            r#"<action content="Dismiss" activationType="system" arguments="dismiss"/>"#,
            r#"</actions></toast>"#,
        ),
        launch = launch,
        title = title,
        text = text,
        open = open,
    )
}

#[cfg(windows)]
mod win {
    use std::ptr;

    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    use super::{desk_toast_xml, AUMID, DESK_GROUP, DESK_TAG, DISPLAY_NAME};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Register the app identity for the current user so Windows will show
    /// its toasts. Safe to repeat; it only rewrites the same value.
    pub fn register_app_id() -> Result<(), String> {
        let path = format!("Software\\Classes\\AppUserModelId\\{AUMID}");
        let mut hkey: HKEY = ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(&path).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                ptr::null(),
                &mut hkey,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!("could not open the app id key (error {status})"));
        }
        let value = wide(DISPLAY_NAME);
        let status = unsafe {
            RegSetValueExW(
                hkey,
                wide("DisplayName").as_ptr(),
                0,
                REG_SZ,
                value.as_ptr() as *const u8,
                (value.len() * std::mem::size_of::<u16>()) as u32,
            )
        };
        unsafe { RegCloseKey(hkey) };
        if status != ERROR_SUCCESS {
            return Err(format!("could not write the app name (error {status})"));
        }
        Ok(())
    }

    /// Show a desk notice as a toast that stays until dismissed, replacing
    /// any desk toast already on screen.
    pub fn show_desk(title: &str, text: &str, link: Option<&str>) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(desk_toast_xml(title, text, link)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetTag(&HSTRING::from(DESK_TAG))?;
        toast.SetGroup(&HSTRING::from(DESK_GROUP))?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?;
        notifier.Show(&toast)
    }
}

#[cfg(windows)]
pub use win::{register_app_id, show_desk};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_drops_control_characters() {
        assert_eq!(
            xml_escape(r#"a<b>&"c"'d'"#),
            "a&lt;b&gt;&amp;&quot;c&quot;&apos;d&apos;"
        );
        assert_eq!(xml_escape("a\u{1}b\nc"), "ab\nc");
        assert_eq!(xml_escape("CO₂ 1400 ppm"), "CO₂ 1400 ppm");
    }

    #[test]
    fn stays_until_dismissed_and_is_silent() {
        let xml = desk_toast_xml("Desk air is stuffy", "Open a window", None);
        assert!(xml.starts_with(r#"<toast scenario="reminder">"#));
        assert!(xml.contains(r#"<audio silent="true"/>"#));
        assert!(xml.contains(r#"activationType="system" arguments="dismiss""#));
        assert!(xml.contains("<text>Desk air is stuffy</text><text>Open a window</text>"));
    }

    #[test]
    fn without_a_link_there_is_no_open_button() {
        let xml = desk_toast_xml("t", "x", None);
        assert!(!xml.contains("Open"));
        assert!(!xml.contains("launch="));
    }

    #[test]
    fn a_link_is_opened_by_a_click_and_by_open() {
        let xml = desk_toast_xml("t", "x", Some("http://192.168.86.25:3006/air?a=1&b=2"));
        let url = "http://192.168.86.25:3006/air?a=1&amp;b=2";
        assert!(xml.contains(&format!(r#"activationType="protocol" launch="{url}""#)));
        assert!(xml.contains(&format!(
            r#"<action content="Open" activationType="protocol" arguments="{url}"/>"#
        )));
    }

    #[test]
    fn text_cannot_break_out_of_the_xml() {
        let xml = desk_toast_xml("</text><evil/>", "\"><x>", None);
        assert!(!xml.contains("<evil/>"));
        assert!(!xml.contains("<x>"));
    }
}

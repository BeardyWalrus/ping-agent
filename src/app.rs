//! The tray application: Win32 UI via native-windows-gui, a background ping
//! thread, and the settings window.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use native_windows_gui as nwg;
use winapi::shared::windef::{HBITMAP, HICON};
use winapi::um::wingdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use winapi::um::winuser::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, ModifyMenuW, SetForegroundWindow, ICONINFO,
    MF_BYCOMMAND, MF_GRAYED, MF_STRING, SM_CXSMICON,
};

use crate::autostart;
use crate::config::{Config, APP_NAME};
use crate::icon::{self, Image, Rgb};
use crate::ping::Pinger;
use crate::stats::{icon_text, outcome_label, PingOutcome, Stats};

/// Number of recent results kept for the average / loss figures.
const HISTORY: usize = 60;

enum Command {
    PingNow,
    ConfigChanged,
    Quit,
}

struct State {
    config: Config,
    stats: Stats,
    /// The HICON currently shown in the tray; destroyed when replaced.
    current_icon: HICON,
    shared_config: Arc<Mutex<Config>>,
    cmd_tx: Option<Sender<Command>>,
    result_rx: Option<Receiver<PingOutcome>>,
    worker: Option<JoinHandle<()>>,
}

impl Default for State {
    fn default() -> Self {
        let config = Config::default();
        State {
            shared_config: Arc::new(Mutex::new(config.clone())),
            config,
            stats: Stats::new(HISTORY),
            current_icon: std::ptr::null_mut(),
            cmd_tx: None,
            result_rx: None,
            worker: None,
        }
    }
}

#[derive(Default)]
pub struct App {
    window: nwg::MessageWindow,
    tray: nwg::TrayNotification,
    notice: nwg::Notice,

    tray_menu: nwg::Menu,
    menu_status: nwg::MenuItem,
    menu_stats: nwg::MenuItem,
    menu_sep1: nwg::MenuSeparator,
    menu_ping_now: nwg::MenuItem,
    menu_settings: nwg::MenuItem,
    menu_sep2: nwg::MenuSeparator,
    menu_exit: nwg::MenuItem,

    settings: nwg::Window,
    host_label: nwg::Label,
    host_input: nwg::TextInput,
    interval_label: nwg::Label,
    interval_input: nwg::TextInput,
    timeout_label: nwg::Label,
    timeout_input: nwg::TextInput,
    warn_label: nwg::Label,
    warn_input: nwg::TextInput,
    bad_label: nwg::Label,
    bad_input: nwg::TextInput,
    autostart_check: nwg::CheckBox,
    path_label: nwg::Label,
    save_button: nwg::Button,
    cancel_button: nwg::Button,

    state: RefCell<State>,
}

pub struct Ui {
    inner: Rc<App>,
    handlers: RefCell<Vec<nwg::EventHandler>>,
}

impl Drop for Ui {
    fn drop(&mut self) {
        for h in self.handlers.borrow_mut().drain(..) {
            nwg::unbind_event_handler(&h);
        }
    }
}

impl std::ops::Deref for Ui {
    type Target = App;
    fn deref(&self) -> &App {
        &self.inner
    }
}

/// Entry point: builds the UI, starts the worker thread, runs the message loop.
pub fn run() {
    // With no console window, a panic would otherwise vanish silently.
    std::panic::set_hook(Box::new(|info| {
        nwg::error_message("PingAgent stopped unexpectedly", &info.to_string());
    }));

    nwg::init().expect("Failed to initialise native-windows-gui");
    if let Err(e) = nwg::Font::set_global_family("Segoe UI") {
        eprintln!("could not set default font: {e}");
    }

    if already_running() {
        nwg::simple_message(
            APP_NAME,
            "PingAgent is already running. Look for it in the system tray.",
        );
        return;
    }

    let (config, load_error) = match Config::load() {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };

    let app = App::default();
    app.state.borrow_mut().config = config.clone();
    *app.state.borrow_mut().shared_config.lock().unwrap() = config;

    let ui = build_ui(app).expect("Failed to build the user interface");
    ui.start_worker();
    ui.refresh();

    if let Some(e) = load_error {
        nwg::error_message(
            APP_NAME,
            &format!("{e}\n\nPingAgent will use its default settings."),
        );
    }

    nwg::dispatch_thread_events();
    ui.shutdown();
}

/// Use a named mutex so a second launch does not create a second tray icon.
fn already_running() -> bool {
    use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::synchapi::CreateMutexW;
    let name: Vec<u16> = "Local\\PingAgent-single-instance\0"
        .encode_utf16()
        .collect();
    unsafe {
        // Intentionally leaked: the mutex must live as long as the process.
        let handle = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        !handle.is_null() && GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn build_ui(mut data: App) -> Result<Ui, nwg::NwgError> {
    use nwg::Event as E;

    // --- Tray -----------------------------------------------------------------
    nwg::MessageWindow::builder().build(&mut data.window)?;

    let placeholder = render_icon("...", icon::NEUTRAL);
    let mut first_icon = nwg::Icon::default();
    first_icon.handle = placeholder as _;

    nwg::TrayNotification::builder()
        .parent(&data.window)
        .icon(Some(&first_icon))
        .tip(Some("PingAgent: starting"))
        .build(&mut data.tray)?;
    data.state.borrow_mut().current_icon = placeholder;

    nwg::Notice::builder()
        .parent(&data.window)
        .build(&mut data.notice)?;

    nwg::Menu::builder()
        .popup(true)
        .parent(&data.window)
        .build(&mut data.tray_menu)?;
    nwg::MenuItem::builder()
        .text("PingAgent")
        .disabled(true)
        .parent(&data.tray_menu)
        .build(&mut data.menu_status)?;
    nwg::MenuItem::builder()
        .text("no data yet")
        .disabled(true)
        .parent(&data.tray_menu)
        .build(&mut data.menu_stats)?;
    nwg::MenuSeparator::builder()
        .parent(&data.tray_menu)
        .build(&mut data.menu_sep1)?;
    nwg::MenuItem::builder()
        .text("Ping &now")
        .parent(&data.tray_menu)
        .build(&mut data.menu_ping_now)?;
    nwg::MenuItem::builder()
        .text("&Settings...")
        .parent(&data.tray_menu)
        .build(&mut data.menu_settings)?;
    nwg::MenuSeparator::builder()
        .parent(&data.tray_menu)
        .build(&mut data.menu_sep2)?;
    nwg::MenuItem::builder()
        .text("E&xit")
        .parent(&data.tray_menu)
        .build(&mut data.menu_exit)?;

    // --- Settings window --------------------------------------------------------
    nwg::Window::builder()
        .flags(nwg::WindowFlags::WINDOW)
        .size((380, 312))
        .center(true)
        .title("PingAgent settings")
        .build(&mut data.settings)?;

    let rows: [(&str, &mut nwg::Label, &mut nwg::TextInput, bool); 5] = [
        (
            "Host name or IP address",
            &mut data.host_label,
            &mut data.host_input,
            false,
        ),
        (
            "Ping every (seconds)",
            &mut data.interval_label,
            &mut data.interval_input,
            true,
        ),
        (
            "Timeout (milliseconds)",
            &mut data.timeout_label,
            &mut data.timeout_input,
            true,
        ),
        (
            "Amber at or above (ms)",
            &mut data.warn_label,
            &mut data.warn_input,
            true,
        ),
        (
            "Red at or above (ms)",
            &mut data.bad_label,
            &mut data.bad_input,
            true,
        ),
    ];
    let mut y = 16;
    for (text, label, input, numeric) in rows {
        nwg::Label::builder()
            .text(text)
            .position((16, y + 3))
            .size((176, 22))
            .parent(&data.settings)
            .build(label)?;
        let mut flags = nwg::TextInputFlags::VISIBLE | nwg::TextInputFlags::TAB_STOP;
        if numeric {
            flags |= nwg::TextInputFlags::NUMBER;
        }
        nwg::TextInput::builder()
            .position((200, y))
            .size((164, 26))
            .flags(flags)
            .parent(&data.settings)
            .build(input)?;
        y += 38;
    }

    nwg::CheckBox::builder()
        .text("Start PingAgent when I sign in to Windows")
        .position((16, y + 2))
        .size((348, 24))
        .parent(&data.settings)
        .build(&mut data.autostart_check)?;
    y += 36;

    let path_text = match Config::path() {
        Some(p) => format!("Settings file: {}", p.display()),
        None => String::from("Settings file: (unavailable)"),
    };
    nwg::Label::builder()
        .text(&path_text)
        .position((16, y))
        .size((348, 20))
        .flags(nwg::LabelFlags::VISIBLE | nwg::LabelFlags::ELIPSIS)
        .parent(&data.settings)
        .build(&mut data.path_label)?;
    y += 30;

    nwg::Button::builder()
        .text("Save")
        .position((196, y))
        .size((80, 30))
        .parent(&data.settings)
        .build(&mut data.save_button)?;
    nwg::Button::builder()
        .text("Cancel")
        .position((284, y))
        .size((80, 30))
        .parent(&data.settings)
        .build(&mut data.cancel_button)?;

    // --- Events -----------------------------------------------------------------
    let ui = Ui {
        inner: Rc::new(data),
        handlers: Default::default(),
    };

    let weak = Rc::downgrade(&ui.inner);
    let tray_events = move |evt, _data, handle: nwg::ControlHandle| {
        let Some(app) = weak.upgrade() else { return };
        match evt {
            E::OnContextMenu | E::OnMousePress(nwg::MousePressEvent::MousePressLeftUp)
                if handle == app.tray =>
            {
                app.show_menu();
            }
            E::OnNotice if handle == app.notice => app.on_results(),
            E::OnMenuItemSelected => {
                if handle == app.menu_ping_now {
                    app.send(Command::PingNow);
                } else if handle == app.menu_settings {
                    app.show_settings();
                } else if handle == app.menu_exit {
                    nwg::stop_thread_dispatch();
                }
            }
            _ => {}
        }
    };
    ui.handlers
        .borrow_mut()
        .push(nwg::full_bind_event_handler(&ui.window.handle, tray_events));

    let weak = Rc::downgrade(&ui.inner);
    let settings_events = move |evt, _data, handle: nwg::ControlHandle| {
        let Some(app) = weak.upgrade() else { return };
        if evt == E::OnButtonClick {
            if handle == app.save_button {
                app.save_settings();
            } else if handle == app.cancel_button {
                app.settings.set_visible(false);
            }
        }
        // OnWindowClose: nwg hides the window by default, which is what we want.
    };
    ui.handlers.borrow_mut().push(nwg::full_bind_event_handler(
        &ui.settings.handle,
        settings_events,
    ));

    Ok(ui)
}

impl App {
    fn send(&self, cmd: Command) {
        if let Some(tx) = &self.state.borrow().cmd_tx {
            let _ = tx.send(cmd);
        }
    }

    fn start_worker(&self) {
        let (cmd_tx, cmd_rx) = channel::<Command>();
        let (result_tx, result_rx) = channel::<PingOutcome>();
        let shared = self.state.borrow().shared_config.clone();
        let notice = self.notice.sender();
        let worker = std::thread::Builder::new()
            .name("ping".into())
            .spawn(move || worker_loop(shared, cmd_rx, result_tx, notice))
            .expect("failed to start the ping thread");
        let mut st = self.state.borrow_mut();
        st.cmd_tx = Some(cmd_tx);
        st.result_rx = Some(result_rx);
        st.worker = Some(worker);
    }

    fn shutdown(&self) {
        self.send(Command::Quit);
        let worker = self.state.borrow_mut().worker.take();
        if let Some(w) = worker {
            // The worker wakes within one ping timeout; don't make exit feel slow.
            let deadline = Instant::now() + Duration::from_millis(1500);
            while !w.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let old = std::mem::replace(
            &mut self.state.borrow_mut().current_icon,
            std::ptr::null_mut(),
        );
        if !old.is_null() {
            unsafe { DestroyIcon(old) };
        }
    }

    /// Called on the UI thread whenever the worker has produced results.
    fn on_results(&self) {
        {
            let mut st = self.state.borrow_mut();
            let State {
                result_rx, stats, ..
            } = &mut *st;
            if let Some(rx) = result_rx {
                while let Ok(outcome) = rx.try_recv() {
                    stats.push(outcome);
                }
            }
        }
        self.refresh();
    }

    /// Redraw the tray icon, tooltip and menu from the current state.
    fn refresh(&self) {
        let (config, summary) = {
            let st = self.state.borrow();
            (st.config.clone(), st.stats.summary())
        };
        let last = summary.last.as_ref();

        let color = match last {
            None => icon::NEUTRAL,
            Some(PingOutcome::Reply(d)) => {
                let ms = d.as_millis() as u64;
                if ms >= config.bad_ms {
                    icon::BAD
                } else if ms >= config.warn_ms {
                    icon::WARN
                } else {
                    icon::GOOD
                }
            }
            Some(_) => icon::BAD,
        };

        let new_icon = render_icon(&icon_text(last), color);
        if !new_icon.is_null() {
            let mut wrapper = nwg::Icon::default();
            wrapper.handle = new_icon as _;
            self.tray.set_icon(&wrapper);
            // The shell copies the icon, so the previous one can go now.
            let old = std::mem::replace(&mut self.state.borrow_mut().current_icon, new_icon);
            if !old.is_null() {
                unsafe { DestroyIcon(old) };
            }
        }

        let status = format!("{}: {}", config.host, outcome_label(last));
        let stats_line = match summary.avg_ms {
            Some(avg) => format!(
                "avg {:.0} ms, min {} ms, max {} ms, loss {:.0}% of {}",
                avg,
                summary.min_ms.unwrap_or(0),
                summary.max_ms.unwrap_or(0),
                summary.loss_pct,
                summary.samples
            ),
            None if summary.samples > 0 => {
                format!("no replies in the last {} pings", summary.samples)
            }
            None => "no data yet".to_string(),
        };

        let mut tip = format!("{status}\n{stats_line}");
        if tip.chars().count() > 127 {
            tip = tip.chars().take(126).collect::<String>() + "…";
        }
        self.tray.set_tip(&tip);

        set_menu_item_text(&self.menu_status, &status, true);
        set_menu_item_text(&self.menu_stats, &stats_line, true);
    }

    fn show_menu(&self) {
        let (x, y) = nwg::GlobalCursor::position();
        self.tray_menu.popup(x, y);
    }

    fn show_settings(&self) {
        let config = self.state.borrow().config.clone();
        self.host_input.set_text(&config.host);
        self.interval_input
            .set_text(&config.interval_secs.to_string());
        self.timeout_input.set_text(&config.timeout_ms.to_string());
        self.warn_input.set_text(&config.warn_ms.to_string());
        self.bad_input.set_text(&config.bad_ms.to_string());
        self.autostart_check
            .set_check_state(if autostart::is_enabled() {
                nwg::CheckBoxState::Checked
            } else {
                nwg::CheckBoxState::Unchecked
            });
        self.settings.set_visible(true);
        if let Some(hwnd) = self.settings.handle.hwnd() {
            unsafe { SetForegroundWindow(hwnd) };
        }
        self.host_input.set_focus();
    }

    fn save_settings(&self) {
        let parse = |input: &nwg::TextInput, what: &str| -> Result<u64, String> {
            input
                .text()
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("{what} must be a whole number."))
        };
        let parsed = (|| -> Result<Config, String> {
            let cfg = Config {
                host: self.host_input.text().trim().to_string(),
                interval_secs: parse(&self.interval_input, "Ping interval")?,
                timeout_ms: parse(&self.timeout_input, "Timeout")?,
                warn_ms: parse(&self.warn_input, "Amber threshold")?,
                bad_ms: parse(&self.bad_input, "Red threshold")?,
            };
            cfg.validate()?;
            Ok(cfg)
        })();
        let config = match parsed {
            Ok(c) => c,
            Err(msg) => {
                nwg::modal_error_message(&self.settings, "PingAgent settings", &msg);
                return;
            }
        };

        let mut problems = Vec::new();
        if let Err(e) = config.save() {
            problems.push(format!("Settings were applied but could not be saved: {e}"));
        }

        let want_autostart = self.autostart_check.check_state() == nwg::CheckBoxState::Checked;
        if want_autostart != autostart::is_enabled() {
            if let Err(e) = autostart::set_enabled(want_autostart) {
                problems.push(format!(
                    "Could not change the start-with-Windows setting: {e}"
                ));
            }
        }

        let host_changed = {
            let mut st = self.state.borrow_mut();
            let host_changed = st.config.host != config.host;
            st.config = config.clone();
            *st.shared_config.lock().unwrap() = config;
            if host_changed {
                st.stats.clear();
            }
            host_changed
        };
        self.send(Command::ConfigChanged);
        if host_changed {
            self.refresh();
        }
        self.settings.set_visible(false);

        if !problems.is_empty() {
            nwg::error_message("PingAgent settings", &problems.join("\n\n"));
        }
    }
}

/// Background loop: ping, report, wait for the interval or a command.
fn worker_loop(
    shared: Arc<Mutex<Config>>,
    cmd_rx: Receiver<Command>,
    result_tx: Sender<PingOutcome>,
    notice: nwg::NoticeSender,
) {
    let mut pinger: Option<Pinger> = None;
    loop {
        let cfg = shared.lock().unwrap().clone();

        if pinger.is_none() {
            match Pinger::new() {
                Ok(p) => pinger = Some(p),
                Err(e) => {
                    if result_tx.send(PingOutcome::Error(e)).is_err() {
                        return;
                    }
                    notice.notice();
                }
            }
        }
        if let Some(p) = &pinger {
            let outcome = p.ping(&cfg.host, Duration::from_millis(cfg.timeout_ms));
            if result_tx.send(outcome).is_err() {
                return;
            }
            notice.notice();
        }

        // Sleep until the next ping, unless a command arrives first. A config
        // change or "ping now" starts the next ping immediately.
        match cmd_rx.recv_timeout(Duration::from_secs(cfg.interval_secs)) {
            Ok(Command::Quit) | Err(RecvTimeoutError::Disconnected) => return,
            Ok(Command::PingNow) | Ok(Command::ConfigChanged) | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Size the shell wants for tray icons at the current DPI.
fn tray_icon_size() -> u32 {
    let n = unsafe { GetSystemMetrics(SM_CXSMICON) };
    (n.max(16) as u32).min(64)
}

fn render_icon(text: &str, color: Rgb) -> HICON {
    let img = icon::render(text, color, tray_icon_size());
    create_hicon(&img)
}

/// Turn an RGBA image into an HICON with a proper alpha channel.
fn create_hicon(img: &Image) -> HICON {
    let size = img.size as i32;
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size,
        biHeight: -size, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let info = BITMAPINFO {
        bmiHeader: header,
        bmiColors: [unsafe { std::mem::zeroed() }],
    };

    unsafe {
        let mut bits: *mut winapi::ctypes::c_void = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(
            std::ptr::null_mut(),
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let pixels = icon::to_premultiplied_bgra(img);
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());

        let mask: HBITMAP = CreateBitmap(size, size, 1, 1, std::ptr::null());
        let mut info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let hicon = CreateIconIndirect(&mut info);
        if !mask.is_null() {
            DeleteObject(mask as _);
        }
        DeleteObject(color as _);
        hicon
    }
}

/// native-windows-gui has no `set_text` for menu items, so talk to Win32 directly.
fn set_menu_item_text(item: &nwg::MenuItem, text: &str, disabled: bool) {
    if let nwg::ControlHandle::MenuItem(parent, id) = item.handle {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let mut flags = MF_BYCOMMAND | MF_STRING;
        if disabled {
            flags |= MF_GRAYED;
        }
        unsafe {
            ModifyMenuW(parent, id, flags, id as usize, wide.as_ptr());
        }
    }
}

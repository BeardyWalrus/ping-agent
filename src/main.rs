#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
// Off Windows only the unit tests are meaningful; most code is unused there.
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod icon;
mod schedule;
mod stats;

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod autostart;
#[cfg(windows)]
mod ping;

#[cfg(windows)]
fn main() {
    app::run();
}

#[cfg(not(windows))]
fn main() {
    eprintln!("PingAgent is a Windows system tray application; this build target only runs the unit tests.");
    std::process::exit(1);
}

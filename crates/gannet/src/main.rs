//! Gannet: a screen capture tool for macOS that lives in the menu bar.
//! Region, window and full screen captures by global shortcuts, marked up
//! on the frozen screen, then copied and saved.

mod capture;
mod hotkeys;
mod overlay;
mod paint;
mod status_item;

use gpui_kit::*;

/// What the menu bar item and the global shortcuts ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Capture(capture::Mode),
    OpenCaptures,
    Quit,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        let (tx, rx) = flume::unbounded::<Command>();
        status_item::install(tx.clone());
        hotkeys::install(tx, cx);
        capture::init(cx);
        cx.spawn(async move |cx| {
            while let Ok(cmd) = rx.recv_async().await {
                cx.update(|cx| run(cmd, cx));
            }
        })
        .detach();
        log::info!("Gannet {} ready", env!("CARGO_PKG_VERSION"));
        // `--capture region|window|screen`: start one right away.
        if let Some(mode) = capture_arg() {
            capture::start(mode, cx);
        }
    });
}

fn capture_arg() -> Option<capture::Mode> {
    let args: Vec<String> = std::env::args().collect();
    let at = args.iter().position(|a| a == "--capture")?;
    match args.get(at + 1).map(String::as_str) {
        Some("window") => Some(capture::Mode::Window),
        Some("screen") => Some(capture::Mode::Screen),
        _ => Some(capture::Mode::Region),
    }
}

fn run(cmd: Command, cx: &mut App) {
    match cmd {
        Command::Capture(mode) => capture::start(mode, cx),
        Command::OpenCaptures => {
            let dir = gannet_core::store::captures_dir();
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::process::Command::new("/usr/bin/open").arg(&dir).spawn();
        },
        Command::Quit => cx.quit(),
    }
}

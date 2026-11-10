//! Gannet: a screen capture tool for macOS that lives in the menu bar.
//! Region, window and full screen captures by global shortcuts, marked up
//! on the frozen screen, then copied and kept, encrypted, in the library
//! until exported.

mod capture;
mod dock;
mod hotkeys;
mod icons;
mod library;
mod menus;
mod open_panel;
mod overlay;
mod paint;
mod permissions;
mod popup;
mod popup_menu;
mod prefs;
mod scroll;
mod settings_window;
mod status_item;
mod toolbar;
mod ui;

use gpui_kit::*;

/// What the menu bar item and the global shortcuts ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Capture(capture::Mode),
    OpenLibrary,
    /// Image files, picked, kept as new captures.
    OpenFiles,
    /// The image on the clipboard, kept as a new capture.
    OpenClipboard,
    OpenSettings,
    /// Show the capture toolbar, or hide it.
    ToggleToolbar,
    /// The menu bar icon clicked: the toolbar under it, shown or hidden.
    ToggleAttachedToolbar,
    /// A click elsewhere: the toolbar under the icon goes.
    CloseAttachedToolbar,
    Quit,
}

/// Where commands go, for the views that send them.
struct Commands(flume::Sender<Command>);

impl Global for Commands {}

/// Where commands go, for AppKit callbacks outside GPUI.
pub fn sender(cx: &App) -> flume::Sender<Command> {
    cx.global::<Commands>().0.clone()
}

/// Run `cmd` after the current update (a click may come from a window the
/// command closes or covers).
pub fn send(cx: &App, cmd: Command) {
    let _ = cx.global::<Commands>().0.send(cmd);
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let app = gpui_kit::application().with_assets(icons::Assets);
    // The Dock icon clicked with no window open: the library (editor).
    app.on_reopen(library::open);
    app.run(|cx| {
        gpui_kit::init(cx);
        let (tx, rx) = flume::unbounded::<Command>();
        status_item::install(tx.clone());
        cx.set_global(Commands(tx.clone()));
        hotkeys::install(tx, cx);
        prefs::init(cx);
        menus::init(cx);
        capture::init(cx);
        library::init(cx);
        cx.spawn(async move |cx| {
            while let Ok(cmd) = rx.recv_async().await {
                cx.update(|cx| run(cmd, cx));
            }
        })
        .detach();
        log::info!("Gannet {} ready", env!("CARGO_PKG_VERSION"));
        // `--capture region|window|screen|scroll`: start one right away.
        if let Some(mode) = capture_arg() {
            capture::start(mode, cx);
        }
        // `--library`: open the library.
        if std::env::args().any(|a| a == "--library") {
            library::open(cx);
        }
        // `--settings`: open the Settings window.
        if std::env::args().any(|a| a == "--settings") {
            settings_window::open(cx);
        }
    });
}

fn capture_arg() -> Option<capture::Mode> {
    let args: Vec<String> = std::env::args().collect();
    let at = args.iter().position(|a| a == "--capture")?;
    match args.get(at + 1).map(String::as_str) {
        Some("window") => Some(capture::Mode::Window),
        Some("screen") => Some(capture::Mode::Screen),
        Some("scroll") => Some(capture::Mode::Scroll),
        _ => Some(capture::Mode::Region),
    }
}

fn run(cmd: Command, cx: &mut App) {
    match cmd {
        Command::Capture(mode) => capture::start(mode, cx),
        Command::OpenLibrary => {
            library::open(cx);
        },
        Command::OpenFiles => library::open_files(cx),
        Command::OpenClipboard => library::open_clipboard(cx),
        Command::OpenSettings => settings_window::open(cx),
        Command::ToggleToolbar => prefs::update(cx, |s| s.toolbar = !s.toolbar),
        Command::ToggleAttachedToolbar => toolbar::toggle_attached(cx),
        Command::CloseAttachedToolbar => toolbar::close_attached(cx),
        Command::Quit => cx.quit(),
    }
}

//! Setting Gannet up, before it runs, in two steps: what it does and how
//! it keeps captures private; then the Screen Recording permission,
//! without which nothing can be captured, the shortcuts and where exports
//! go. macOS applies the permission when Gannet starts, so Gannet reopens
//! to start.
//!
//! A later start without the permission (taken back, say) only asks for
//! it, and reopens by itself once it is given.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gannet_core::capture as grab;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::icons::{IconName, icon};
use crate::settings_window::{Embedded, Pal, SettingsView, is_dark, push_button};
use crate::{dock, menus, paint, permissions, prefs};

/// How often the last step looks whether the permission was given.
const CHECK_EVERY: Duration = Duration::from_secs(1);
const WIDTH: f32 = 600.;
const HEIGHT: f32 = 640.;
/// Asking only for the permission: the icon, a heading and its row.
const HEIGHT_PERMISSION: f32 = 330.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Welcome,
    Setup,
}

/// `--onboarding`: every step, as on a first start, whatever is set up.
fn forced() -> bool {
    std::env::args().any(|a| a == "--onboarding")
}

/// Whether Gannet has to be set up before it runs: a first start, or no
/// Screen Recording.
pub fn needed() -> bool {
    forced() || !prefs::set_up_before() || !grab::has_permission()
}

/// Show the window; `start` runs Gannet as usual, for a build that can't
/// reopen itself (`cargo run`) once the permission is given.
pub fn show(start: fn(&mut App), cx: &mut App) {
    // The settings the steps change, not put to work yet: no shortcut is
    // registered and no toolbar shown until Gannet runs.
    prefs::load(cx);
    cx.bind_keys([KeyBinding::new("cmd-q", menus::Quit, None), KeyBinding::new("cmd-w", menus::CloseWindow, None)]);
    cx.on_action(|_: &menus::Quit, cx| cx.quit());
    cx.set_menus([Menu::new("Gannet").items([MenuItem::action("Quit Gannet", menus::Quit)])]);
    dock::set(dock::Window::Onboarding, true, cx);
    cx.activate(true);
    // Set up before, only the permission is missing.
    let permission_only = !forced() && prefs::set_up_before();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(WIDTH), px(if permission_only { HEIGHT_PERMISSION } else { HEIGHT })),
            cx,
        ))),
        titlebar: Some(TitlebarOptions { title: Some("Welcome to Gannet".into()), appears_transparent: true, ..Default::default() }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    let opened = gpui_kit::open_window(options, cx, move |window, cx| {
        // Closing the window is quitting: there is nothing else to Gannet yet.
        window.on_window_should_close(cx, |_, cx| {
            cx.quit();
            true
        });
        cx.new(|cx| Onboarding::new(permission_only, start, window, cx))
    });
    if let Err(e) = opened {
        log::error!("can't open the onboarding window: {e}");
        start(cx);
    }
}

struct Onboarding {
    step: Step,
    /// Only the permission is asked for (Gannet was set up before).
    permission_only: bool,
    /// Screen Recording is allowed (as macOS says now; it may only say so
    /// once Gannet is reopened, and then macOS offers to reopen it).
    allowed: bool,
    shortcuts: Entity<SettingsView>,
    folder: Entity<SettingsView>,
    start: fn(&mut App),
    _check: Task<()>,
}

impl Onboarding {
    fn new(permission_only: bool, start: fn(&mut App), window: &mut Window, cx: &mut Context<Self>) -> Self {
        let shortcuts = cx.new(|cx| SettingsView::embedded(Embedded::Shortcuts, window, cx));
        let folder = cx.new(|cx| SettingsView::embedded(Embedded::Folder, window, cx));
        // Whether it is allowed yet. Asked for alone, Gannet then reopens
        // by itself, a moment after the window has said so; with the
        // settings beside it, the user starts it when done with them.
        let check = cx.spawn_in(window, async move |this, cx| {
            loop {
                let allowed = grab::has_permission();
                let Ok(go) = this.update(cx, |this, cx| {
                    if this.allowed != allowed {
                        this.allowed = allowed;
                        cx.notify();
                    }
                    allowed && this.permission_only
                }) else {
                    return;
                };
                if go {
                    cx.background_executor().timer(CHECK_EVERY).await;
                    let _ = this.update_in(cx, |this, window, cx| this.finish(window, cx));
                    return;
                }
                cx.background_executor().timer(CHECK_EVERY).await;
            }
        });
        let step = if permission_only { Step::Setup } else { Step::Welcome };
        Self { step, permission_only, allowed: false, shortcuts, folder, start, _check: check }
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.step = step;
        cx.notify();
    }

    /// Set up: the settings kept (so this isn't shown again), and Gannet
    /// reopened so the permission takes; a build that isn't an app bundle
    /// carries on in place instead.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        prefs::save(cx);
        // Carries on in place if it can't be reopened.
        if permissions::relaunch(false, cx) {
            return;
        }
        let start = self.start;
        dock::set(dock::Window::Onboarding, false, cx);
        window.remove_window();
        cx.defer(start);
    }
}

/// Gannet's icon, as GPUI shows it (decoded once).
fn app_icon() -> Option<Arc<RenderImage>> {
    static ICON: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
    let img = ICON.get_or_init(|| image::load_from_memory(include_bytes!("../resources/AppIcon.png")).ok().map(|i| i.into_rgba8()));
    img.as_ref().map(paint::picture)
}

/// A step's heading: a title, and a line under it.
fn heading(title: &str, line: &str, pal: Pal) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child(title.to_owned()))
        .child(div().text_size(px(12.)).text_color(pal.secondary).text_center().child(line.to_owned()))
}

/// What Gannet does, one line: an icon, a name, a few words.
fn feature(name: IconName, title: &str, line: &str, pal: Pal) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(div().flex_none().size(px(30.)).rounded(px(7.)).bg(pal.button).flex().items_center().justify_center().child(icon(name, 16.)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(title.to_owned()))
                .child(div().text_size(px(11.)).text_color(pal.secondary).child(line.to_owned())),
        )
}

/// A rounded box, as Settings groups its rows.
fn panel(pal: Pal) -> Div {
    div().w_full().rounded(px(10.)).bg(pal.group).border_1().border_color(pal.group_border)
}

/// The button that goes on, drawn strong.
fn primary(id: &'static str, label: &str, pal: Pal) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .h(px(26.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(pal.strong)
        .text_color(pal.on_strong)
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_default()
        .active(|s| s.opacity(0.85))
        .child(label.to_owned())
}

/// Said once the permission is given: a green dot and a word.
fn allowed_badge(pal: Pal) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(pal.secondary)
        .child(div().size(px(8.)).rounded_full().bg(pal.success))
        .child("Allowed")
        .into_any_element()
}

/// Gannet's icon, centred over a step.
fn icon_row() -> AnyElement {
    let icon_img = app_icon().map(|pic| img(ImageSource::Render(pic)).size(px(64.)));
    div().flex().justify_center().children(icon_img).into_any_element()
}

impl Onboarding {
    fn welcome(&self, pal: Pal) -> Vec<AnyElement> {
        // Private by design, set apart from the rest.
        let private = panel(pal)
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(pal.success)
                    .child(icon(IconName::ShieldCheck, 16.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("Private by design")),
            )
            .child(feature(
                IconName::Lock,
                "Encrypted on your Mac",
                "Every capture is kept with AES-256, its key in your Keychain. Only exports are plain files.",
                pal,
            ))
            .child(feature(
                IconName::CloudOff,
                "Nothing leaves this Mac",
                "No account, no cloud, no tracking: Gannet never sends your captures anywhere.",
                pal,
            ));
        vec![
            icon_row(),
            heading("Welcome to Gannet", "Capture your screen, mark it up, and keep it to yourself.", pal).into_any_element(),
            div()
                .w_full()
                .px(px(4.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(feature(IconName::SquareDashed, "Any part of the screen", "A region, a window, or every display.", pal))
                .child(feature(IconName::ScrollText, "Long pages too", "Scroll a region and Gannet stitches it into one picture.", pal))
                .child(feature(IconName::Layers, "Mark it up", "Arrows, text, shapes and blur, on layers you can change later.", pal))
                .into_any_element(),
            private.into_any_element(),
        ]
    }

    /// The Screen Recording row: what to do, and the way to it until it
    /// is allowed.
    fn permission_row(&self, pal: Pal) -> AnyElement {
        let allowed = self.allowed;
        let right = if allowed {
            allowed_badge(pal)
        } else {
            push_button("open-settings", pal)
                .flex_none()
                .child("Open Settings…")
                .on_click(|_, _, _| permissions::ask_screen_recording())
                .into_any_element()
        };
        let detail = match (allowed, self.permission_only) {
            (true, true) => "Allowed. Gannet is starting…",
            (true, false) => "Allowed.",
            (false, _) => "Turn on Gannet in Privacy & Security ▸ Screen & System Audio Recording.",
        };
        panel(pal)
            .px(px(16.))
            .py(px(14.))
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("Screen Recording"))
                    .child(div().text_size(px(11.)).text_color(pal.secondary).child(detail)),
            )
            .child(right)
            .into_any_element()
    }

    fn setup(&self, pal: Pal) -> Vec<AnyElement> {
        if self.permission_only {
            return vec![
                icon_row(),
                heading(
                    "Allow Gannet to record your screen",
                    "macOS asks for your OK before any app can see the screen. Gannet starts as soon as it's given.",
                    pal,
                )
                .into_any_element(),
                self.permission_row(pal),
            ];
        }
        vec![
            heading("Set up Gannet", "Allow it to see the screen, and check the keys and the folder it uses.", pal).into_any_element(),
            section("Permission", pal),
            self.permission_row(pal),
            section("Shortcuts", pal),
            self.shortcuts.clone().into_any_element(),
            section("Exports", pal),
            self.folder.clone().into_any_element(),
        ]
    }
}

/// A section's name over its box.
fn section(name: &str, pal: Pal) -> AnyElement {
    div()
        .px(px(4.))
        .mb(px(-8.))
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(pal.secondary)
        .child(name.to_owned())
        .into_any_element()
}

impl Render for Onboarding {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = Pal::new(is_dark(window));
        let content: Vec<AnyElement> = match self.step {
            Step::Welcome => self.welcome(pal),
            Step::Setup => self.setup(pal),
        };
        // Where the user is: a dot per step.
        let dots = (!self.permission_only).then(|| {
            div().flex().gap(px(6.)).children(
                [Step::Welcome, Step::Setup]
                    .into_iter()
                    .map(|s| div().size(px(6.)).rounded_full().bg(if s == self.step { pal.text } else { pal.tertiary })),
            )
        });
        let (back, next) = match (self.step, self.permission_only) {
            (Step::Welcome, _) => {
                (None, Some(primary("next", "Continue", pal).on_click(cx.listener(|this, _, _, cx| this.go(Step::Setup, cx)))))
            },
            // Asked for alone, the permission starts Gannet by itself.
            (Step::Setup, true) => (None, None),
            (Step::Setup, false) => {
                let back = push_button("back", pal).child("Back").on_click(cx.listener(|this, _, _, cx| this.go(Step::Welcome, cx)));
                // Gannet can't capture until it is allowed.
                let start = primary("start", "Start Gannet", pal);
                let start = if self.allowed {
                    start.on_click(cx.listener(|this, _, window, cx| this.finish(window, cx)))
                } else {
                    start.opacity(0.4)
                };
                (Some(back), Some(start))
            },
        };
        let footer = dots.is_some() || back.is_some() || next.is_some();
        div()
            .key_context("Onboarding")
            .on_action(|_: &menus::CloseWindow, _, cx| cx.quit())
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.window)
            .text_color(pal.text)
            .font_family(".SystemUIFont")
            // The step, scrolling if it ever runs taller than the window, so
            // the buttons under it always show.
            // Asked for alone, the permission sits in the middle, as far
            // from the bottom as from the sides.
            .child(
                div().id("step").flex_1().min_h_0().overflow_y_scroll().when(!footer, |d| d.flex().flex_col().justify_center()).child(
                    div()
                        .flex()
                        .flex_col()
                        .px(px(32.))
                        .pt(px(if footer { 36. } else { 32. }))
                        .pb(px(if footer { 8. } else { 32. }))
                        .gap(px(16.))
                        .children(content),
                ),
            )
            // The buttons, if the step has any.
            .when(footer, |d| {
                d.child(
                    div()
                        .flex_none()
                        .w_full()
                        .px(px(32.))
                        .pt(px(8.))
                        .pb(px(20.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(dots)
                        .child(div().flex_1())
                        .children(back)
                        .children(next),
                )
            })
    }
}

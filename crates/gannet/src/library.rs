//! The capture library: every capture is kept here, encrypted, until the
//! user exports it. Its window fills the screen with the captures by day.
//! A click opens one in the editor, in the same window, to mark it up,
//! crop it, zoom in, copy, export to a real file or delete; ⌘-click and
//! ⇧-click pick several to copy, export or delete at once. A capture in
//! the editor is a stack of layers on a canvas (the picture as taken, the
//! images pasted, the shapes drawn), listed on the editor's right: picked
//! there or on the canvas, to move, resize, restyle, hide, reorder or
//! delete.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gannet_core::edits::{self, Edits, Images, Op};
use gannet_core::folders::Folders;
use gannet_core::model::{self, COLORS, Doc, Grip, ORIGINAL, P, Shape, Style, Tool, p};
use gannet_core::raster;
use gannet_core::settings::Action;
use gannet_core::store;
use gannet_core::vault::{self, Editable, Entry, Vault};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use image::RgbaImage;

use crate::icons::{IconName, icon};
use crate::paint::{self, View, from_point, hsla, picture, to_point};
use crate::ui::{self, BG, DIM, LINE, PANEL, TEXT, button, icon_button, pill, sep, size_dot, swatch, tip, tool_label};
use crate::{capture, dock, open_panel, prefs, toolbar};

/// The window's title while no capture is open.
const TITLE: &str = "Gannet Library";
/// The editor's toolbar, across the top of the window.
const HEADER: f32 = 52.;
/// The editor's toolbar floats over the top of the stage: its top, its
/// height, and the room kept for it above the picture.
const BAR_TOP: f32 = 10.;
const BAR_H: f32 = 40.;
const TOOLBAR: f32 = BAR_TOP + BAR_H;
/// The editor's list of captures, on its left, as a tree of folders.
const SIDE: f32 = 264.;
/// The editor's list of layers, on its right when shown, as wide as the
/// list of captures.
const LAYERS: f32 = SIDE;
const LAYERS_TOP: f32 = 40.;
/// A capture's picture in the side list's rows.
const ROW_PIC_W: f32 = 84.;
const ROW_PIC_H: f32 = 48.;
/// How far the panels floating over the stage keep from its edges.
const FLOAT: f32 = 10.;
const LAYER_ROW: f32 = 32.;
/// Room around the picture in the editor.
const PAD: f32 = 24.;
/// A grid card's width; its picture keeps the capture's shape.
const CARD_W: f32 = 240.;
const CARD_PIC_H: f32 = 150.;
/// The room between cards in the grid, across and down.
const CARD_GAP: f32 = 12.;
/// How many thumbnails are kept decoded; those not shown lately go first.
const THUMBS_KEPT: usize = 240;
/// How many thumbnails are read before they are shown, together.
const THUMB_BATCH: usize = 12;
/// Zoom limits, in window points per image pixel.
const MIN_K: f32 = 0.02;
const MAX_K: f32 = 32.;
/// One zoom step (⌘+, ⌘−).
const ZOOM_STEP: f32 = 1.25;
/// How near a frame's handle a press takes hold of it, in window points.
const GRIP_REACH: f32 = 7.;
/// A frame handle's edge, in window points.
const HANDLE: f32 = 7.;
/// Held while the library's files of a capture change, so a save still
/// going doesn't cross the capture's next opening; with, for each capture,
/// the latest save written, so an older one finishing late is dropped.
static VAULT_WRITES: std::sync::Mutex<Option<HashMap<String, u64>>> = std::sync::Mutex::new(None);
/// Counts saves, so they can be told apart by age.
static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// How long after the last change the editor saves.
const SAVE_AFTER: Duration = Duration::from_secs(1);
/// How often old captures are looked for.
const PRUNE_EVERY: Duration = Duration::from_secs(3600);

#[cfg(not(debug_assertions))]
const KEYCHAIN_SERVICE: &str = "dev.gannet";
#[cfg(not(debug_assertions))]
const KEYCHAIN_ACCOUNT: &str = "library-key";
const KEY_CONTEXT: &str = "GannetLibrary";

/// The library, opened on first use (the Keychain may ask first, so never
/// on the main thread).
pub fn vault() -> Result<Arc<Vault>, String> {
    static VAULT: OnceLock<Result<Arc<Vault>, String>> = OnceLock::new();
    VAULT.get_or_init(open_vault).clone()
}

#[cfg(not(debug_assertions))]
fn open_vault() -> Result<Arc<Vault>, String> {
    key().map(|k| Arc::new(Vault::new(vault::library_dir(), &zeroize::Zeroizing::new(k))))
}

/// Debug builds keep a library of their own, with its key in a file: the
/// Keychain trusts a binary by its signature, and an unsigned one that
/// changes on every build would be asked about each time.
#[cfg(debug_assertions)]
fn open_vault() -> Result<Arc<Vault>, String> {
    let dir = vault::library_dir().with_file_name("library-dev");
    let key = zeroize::Zeroizing::new(vault::file_key(&dir.join("dev.key"))?);
    Ok(Arc::new(Vault::new(dir, &key)))
}

/// The library's key from the Keychain, made on the first start.
#[cfg(not(debug_assertions))]
fn key() -> Result<[u8; 32], String> {
    use security_framework::passwords::{get_generic_password, set_generic_password};
    const NOT_FOUND: i32 = -25300; // errSecItemNotFound
    match get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        Ok(bytes) => {
            // Wiped once copied out, like every copy of the key.
            let bytes = zeroize::Zeroizing::new(bytes);
            <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| "the library key in the Keychain is damaged".to_owned())
        },
        Err(e) if e.code() == NOT_FOUND => {
            let k = vault::new_key();
            set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, &k).map_err(|e| format!("can't keep the library key: {e}"))?;
            Ok(k)
        },
        Err(e) => Err(format!("can't read the library key from the Keychain: {e}")),
    }
}

/// The private folder captures copied together go in, as plain PNGs.
fn copied_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("gannet-copied")
}

/// Delete the captures copied as files: they are decrypted, and outlive
/// the library's own files otherwise.
fn forget_copied() {
    let _ = std::fs::remove_dir_all(copied_dir());
}

/// How often the clipboard is looked at while copied files lie around.
const COPIED_CHECK: Duration = Duration::from_secs(20);

/// The pasteboard's change count, which goes up whenever anything is put
/// on it.
fn pasteboard_count() -> isize {
    objc2_app_kit::NSPasteboard::generalPasteboard().changeCount()
}

/// Start dropping captures older than the settings keep, now and hourly.
pub fn init(cx: &mut App) {
    // Files from a copy before a crash, or before the last quit.
    cx.background_executor().spawn(async { forget_copied() }).detach();
    cx.on_app_quit(|_| {
        forget_copied();
        async {}
    })
    .detach();
    cx.spawn(async move |cx| {
        loop {
            let days = cx.update(|cx| prefs::get(cx).keep_days);
            if let Some(days) = days {
                let max_age = i64::from(days) * 86_400_000;
                let now = chrono::Utc::now().timestamp_millis();
                let gone = cx.background_executor().spawn(async move { vault().map(|v| v.prune(max_age, now)) }).await;
                match gone {
                    Ok(0) => {},
                    Ok(n) => {
                        log::info!("deleted {n} captures older than {days} days");
                        cx.update(refresh);
                    },
                    Err(e) => log::warn!("can't tidy the library: {e}"),
                }
            }
            // And what has been in the trash too long.
            if let Some(days) = cx.update(|cx| prefs::get(cx).trash_days) {
                let max_age = i64::from(days) * 86_400_000;
                let now = chrono::Utc::now().timestamp_millis();
                match cx.background_executor().spawn(async move { vault()?.empty_trash(max_age, now) }).await {
                    Ok(0) => {},
                    Ok(n) => {
                        log::info!("emptied {n} captures from the trash");
                        cx.update(refresh);
                    },
                    Err(e) => log::warn!("can't empty the trash: {e}"),
                }
            }
            cx.background_executor().timer(PRUNE_EVERY).await;
        }
    })
    .detach();
}

/// Keep `img` (`scale` pixels per point) in the library, and open it in
/// the editor with `edit` or if the settings say so.
pub fn keep(img: RgbaImage, scale: f32, edit: bool, cx: &mut App) {
    keep_encoded(img, None, scale, edit, cx);
}

/// Like [`keep`], `png` being the image already encoded, if it is.
pub fn keep_encoded(img: RgbaImage, png: Option<Vec<u8>>, scale: f32, edit: bool, cx: &mut App) {
    cx.spawn(async move |cx| {
        let added = cx
            .background_executor()
            .spawn(async move {
                let v = vault()?;
                match png {
                    Some(png) => v.add_encoded(&img, &png, scale),
                    None => v.add(&img, scale),
                }
            })
            .await;
        cx.update(|cx| match added {
            Ok(e) => {
                log::info!("kept capture {}", e.id);
                refresh(cx);
                if edit || prefs::get(cx).open_editor {
                    show(e, cx);
                }
            },
            Err(e) => capture::notify("Couldn't keep the capture", &e, cx),
        });
    })
    .detach();
}

/// The open window, if there is one. Its root view is gpui-kit's `Root`,
/// not the library, so the library itself is kept beside the handle.
#[derive(Default)]
struct Open(Option<Shown>);

#[derive(Clone)]
struct Shown {
    window: AnyWindowHandle,
    library: WeakEntity<Library>,
}

impl Global for Open {}

impl Shown {
    /// Run `f` on the library in its window; `None` once it is closed.
    fn update<R>(&self, cx: &mut App, f: impl FnOnce(&mut Library, &mut Window, &mut Context<Library>) -> R) -> Option<R> {
        let library = self.library.upgrade()?;
        self.window.update(cx, |_, window, cx| library.update(cx, |this, cx| f(this, window, cx))).ok()
    }
}

/// Read the list again in the open window.
fn refresh(cx: &mut App) {
    if let Some(shown) = cx.try_global::<Open>().and_then(|o| o.0.clone()) {
        shown.update(cx, |this, _, cx| this.reload(cx));
    }
}

/// The library window was put away for a capture.
#[derive(Default)]
struct Aside(bool);

impl Global for Aside {}

/// Put the library window away for a capture. Whether it was on screen.
pub fn step_aside(cx: &mut App) -> bool {
    let Some(shown) = cx.try_global::<Open>().and_then(|o| o.0.clone()) else { return false };
    let Some(window) = toolbar::ns_window(shown.window, cx) else { return false };
    if !window.isVisible() {
        return false;
    }
    window.orderOut(None);
    cx.set_global(Aside(true));
    true
}

/// Bring the library window back after a capture, if it was put away.
pub fn come_back(cx: &mut App) {
    if !cx.try_global::<Aside>().is_some_and(|a| a.0) {
        return;
    }
    cx.set_global(Aside(false));
    let Some(shown) = cx.try_global::<Open>().and_then(|o| o.0.clone()) else { return };
    let Some(window) = toolbar::ns_window(shown.window, cx) else { return };
    cx.activate(true);
    window.makeKeyAndOrderFront(None);
}

/// Show the library, opening it if it isn't open.
pub fn open(cx: &mut App) {
    show_window(cx);
}

/// Show the library window, opening it if it isn't open.
fn show_window(cx: &mut App) -> Option<Shown> {
    // A regular app before it is active, or the menu bar keeps the menus
    // of the app before.
    dock::set(dock::Window::Library, true, cx);
    cx.activate(true);
    if let Some(shown) = cx.try_global::<Open>().and_then(|o| o.0.clone())
        && shown.update(cx, |_, window, _| window.activate_window()).is_some()
    {
        return Some(shown);
    }
    let display = cx.primary_display()?;
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(display.visible_bounds())),
        titlebar: Some(TitlebarOptions { title: Some(TITLE.into()), ..Default::default() }),
        display_id: Some(display.id()),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        window_min_size: Some(size(px(720.), px(420.))),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Library::new(window, cx))) {
        Ok((window, library)) => {
            let shown = Shown { window, library: library.downgrade() };
            cx.set_global(Open(Some(shown.clone())));
            Some(shown)
        },
        Err(e) => {
            log::error!("can't open the library: {e}");
            None
        },
    }
}

/// Open the library on capture `entry`, in the editor.
fn show(entry: Entry, cx: &mut App) {
    if let Some(shown) = show_window(cx) {
        shown.update(cx, |this, window, cx| {
            this.expanded = false;
            this.switch_to(entry, window, cx);
        });
    }
}

/// Keep the image on the clipboard as a new capture, and open it.
pub fn open_clipboard(cx: &mut App) {
    let Some(img) = clipboard_image(cx) else {
        return capture::notify("Nothing to open", "The clipboard has no image.", cx);
    };
    keep(img, capture::main_screen_scale(), true, cx);
}

/// Keep image files the user picks as new captures, and open the last.
pub fn open_files(cx: &mut App) {
    let (tx, rx) = flume::bounded(1);
    open_panel::choose_images(tx);
    let scale = capture::main_screen_scale();
    cx.spawn(async move |cx| {
        let Ok(paths) = rx.recv_async().await else { return };
        let added = cx
            .background_executor()
            .spawn(async move {
                let v = vault()?;
                let mut last = None;
                let mut failed = Vec::new();
                for path in &paths {
                    match store::open(path) {
                        Ok(img) => last = Some(v.add(&img, scale)?),
                        Err(_) => failed.push(path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())),
                    }
                }
                Ok::<_, String>((last, failed))
            })
            .await;
        cx.update(|cx| match added {
            Ok((last, failed)) => {
                if !failed.is_empty() {
                    capture::notify("Some files couldn't be opened", &format!("Not an image Gannet can read: {}", failed.join(", ")), cx);
                }
                refresh(cx);
                if let Some(e) = last {
                    show(e, cx);
                }
            },
            Err(e) => capture::notify("Couldn't open the files", &e, cx),
        });
    })
    .detach();
}

pub struct Library {
    focus: FocusHandle,
    entries: Vec<Entry>,
    /// Thumbnails decoded, by capture; only those shown lately are kept.
    thumbs: HashMap<ThumbKey, Thumb>,
    /// The thumbnails read or being read.
    asked: HashSet<ThumbKey>,
    /// Thumbnails rows in sight want, not read yet.
    wanted: std::cell::RefCell<Vec<ThumbKey>>,
    /// Thumbnails are being read.
    fetching: bool,
    /// Counts frames, to tell which thumbnails were shown lately.
    frame: u64,
    error: Option<String>,
    editor: Option<Editor>,
    /// The id being opened in the editor.
    opening: Option<String>,
    /// The captures picked in the grid, in the order picked.
    selected: Vec<String>,
    /// Where a ⇧-click range starts.
    anchor: Option<usize>,
    /// The side list's rows as last laid out, and its scrolling.
    side_rows: std::cell::RefCell<Rc<Vec<SideRow>>>,
    side_list: ListState,
    /// Likewise for the grid of every capture.
    grid_rows: std::cell::RefCell<Rc<Vec<GridRow>>>,
    grid_list: ListState,
    /// What each list's rows were last made from.
    side_key: std::cell::RefCell<Option<RowsKey>>,
    grid_key: std::cell::RefCell<Option<RowsKey>>,
    /// Counts the times the captures were listed (and the folders read).
    listed: u64,
    /// The window's width at the last frame, for the grid's columns.
    window_width: std::cell::Cell<f32>,
    /// Scroll the side list to the open capture at the next frame.
    reveal: std::cell::Cell<bool>,
    /// The editor's changes, saved a moment after the last one.
    save_soon: Option<Task<()>>,
    /// The window's title as last set: the open capture's date and size.
    title: String,
    /// The side list grown to fill the window, over the editor.
    expanded: bool,
    /// The list has been read at least once.
    loaded: bool,
    /// The editor shows its layers.
    layers_shown: bool,
    /// The folders the captures are sorted into, read once.
    folders: Folders,
    folders_loaded: bool,
    /// The folders closed in the side list's tree.
    folded: HashSet<u64>,
    /// A folder being renamed.
    renaming: Option<Renaming>,
    /// The captures in the trash, latest first, and whether its node in
    /// the side list is open.
    trashed: Vec<Entry>,
    trash_open: bool,
}

/// A thumbnail wanted: its capture, and whether whole (for a card) or
/// small (for a row of the side list).
type ThumbKey = (String, bool);

/// Thumbnail `img` as GPUI shows it, `big` or small, and its size.
fn shown_thumb(img: RgbaImage, big: bool) -> (Arc<RenderImage>, (u32, u32)) {
    // A row's picture, at two pixels a point.
    let (mw, mh) = ((ROW_PIC_W * 2.) as u32, (ROW_PIC_H * 2.) as u32);
    let (w, h) = img.dimensions();
    let img = if !big && (w > mw || h > mh) {
        let k = (mw as f32 / w as f32).min(mh as f32 / h as f32);
        image::imageops::thumbnail(&img, ((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1))
    } else {
        img
    };
    let size = img.dimensions();
    (paint::picture_owned(img), size)
}

/// A thumbnail decoded, as GPUI shows it.
struct Thumb {
    picture: Arc<RenderImage>,
    size: (u32, u32),
    /// The frame it was last shown in.
    shown: std::cell::Cell<u64>,
}

/// A row of the side list.
#[derive(Clone, PartialEq)]
enum SideRow {
    /// A folder's title, with its captures (newest first).
    Folder {
        id: u64,
        name: String,
        entries: Rc<Vec<Entry>>,
    },
    /// A capture in an open folder; `at` is its place in the list.
    Capture {
        at: usize,
        entry: Entry,
    },
    TrashHeader,
    /// A capture in the open trash.
    Trashed(Entry),
    /// Room at the bottom.
    End,
}

/// A row of the grid of every capture.
#[derive(Clone, PartialEq)]
enum GridRow {
    Folder {
        id: u64,
        name: String,
        entries: Rc<Vec<Entry>>,
    },
    /// Cards across, each with its place in the list.
    Cards(Vec<(usize, Entry)>),
    End,
}

/// What a list of captures is laid out from (see [`Library::rows_key`]).
#[derive(PartialEq)]
struct RowsKey {
    listed: u64,
    folders: u64,
    folded: Vec<u64>,
    trash_open: bool,
    cols: usize,
}

/// Lay out `rows` in the list `state` shows, if they differ from those
/// laid out last (`last`); where it was scrolled to is kept.
fn sync_list<R: PartialEq>(state: &ListState, last: &std::cell::RefCell<Rc<Vec<R>>>, rows: Vec<R>) {
    if **last.borrow() == rows {
        return;
    }
    let top = state.logical_scroll_top();
    let n = rows.len();
    *last.borrow_mut() = Rc::new(rows);
    state.reset(n);
    state.scroll_to(ListOffset { item_ix: top.item_ix.min(n.saturating_sub(1)), offset_in_item: top.offset_in_item });
}

/// A small icon button in the trash's rows.
fn trash_action(id: String, name: IconName, hint: &'static str) -> Stateful<Div> {
    div()
        .id(SharedString::from(id))
        .flex_none()
        .text_color(rgb(DIM))
        .hover(|s| s.text_color(rgb(TEXT)))
        .tooltip(tip(hint))
        .child(icon(name, 13.))
}

/// A folder's name being edited in place.
struct Renaming {
    id: u64,
    input: Entity<InputState>,
    _sub: Subscription,
}

/// Captures being dragged to a folder: their ids, and what the pointer
/// carries.
#[derive(Clone)]
struct Dragged {
    ids: Vec<String>,
    label: SharedString,
}

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(rgba(ui::BAR_BG))
            .border_1()
            .border_color(rgba(ui::BAR_LINE))
            .text_size(px(12.))
            .text_color(rgb(TEXT))
            .child(self.label.clone())
    }
}

/// A folder to drop captures in: an id, or none for the captures not in
/// one yet.
const LOOSE: u64 = u64::MAX;

/// The name of the folder a capture goes in by default: its day.
fn day_folder(e: &Entry) -> String {
    e.taken().format("%-d %b %Y").to_string()
}

/// A capture open in the editor: a stack of layers on a canvas, the
/// picture as taken at the bottom.
struct Editor {
    entry: Entry,
    /// The capture as taken.
    original: Arc<RgbaImage>,
    /// The layers, bottom first, as the steps in `edits` make them.
    doc: Doc,
    /// The canvas's size, in pixels.
    size: (u32, u32),
    /// The image layers' pixels as GPUI shows them, by number.
    pictures: HashMap<u32, Arc<RenderImage>>,
    /// The image layers alone, flattened, for a mosaic to show averaging;
    /// only made while there is a mosaic.
    under: Option<Arc<RgbaImage>>,
    /// What `under` was made from: the canvas's size and its image layers
    /// shown. Only a change to them makes it again, not every step.
    under_key: Option<((u32, u32), Vec<Shape>)>,
    tool: Option<Tool>,
    style: Style,
    drag: Option<Shape>,
    text: Option<TextEdit>,
    /// Picking an area instead of drawing, and the area so far (canvas
    /// pixels). A crop's area may run past the canvas, to add room.
    pick: Option<Pick>,
    area: Option<(P, P)>,
    /// An image pasted in and not laid down yet.
    pasted: Option<Pasted>,
    /// A drag of the area, the pasted image, or a picked image's handle.
    frame_drag: Option<FrameDrag>,
    /// The handle under the pointer.
    hover: Option<Grip>,
    /// What was done to the capture, kept with it in the library; what is
    /// shown is these steps played over the capture as it was taken.
    edits: Edits,
    /// The images pasted in, by number.
    images: Images,
    /// Whether anything was done or undone since it was opened.
    changed: bool,
    /// Window points per canvas pixel; `None` fits the window.
    zoom: Option<f32>,
    /// The canvas's offset from centred, in window points.
    pan: P,
    /// A drag moving the canvas: where it began, and the pan then.
    panning: Option<(P, P)>,
    /// The layer picked, by its place in `doc` (the bottom one is 0).
    layer: Option<usize>,
    /// A drag moving the picked layer.
    moving: Option<Moving>,
    /// The pointer is over a layer a drag would move.
    over_layer: bool,
    /// The picked image being resized: the rectangle it is dragged to.
    resize: Option<(P, P)>,
    /// A crop by the canvas's edge started while selecting: selecting
    /// goes on after it.
    select_after: bool,
}

/// A drag moving a layer: where it began, and the layer then.
struct Moving {
    from: P,
    was: Shape,
    /// Far enough to be a move; a click alone only picks the layer.
    moved: bool,
}

/// What a drag over the picture picks instead of drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Crop,
    Select,
}

/// An image pasted over the capture, still moved and resized as a whole:
/// where it lies, in image pixels.
struct Pasted {
    img: Arc<RgbaImage>,
    picture: Arc<RenderImage>,
    rect: (P, P),
}

/// A drag of a frame (the area, or a pasted image): what it holds (`None`
/// draws a new area from `from`), where it began and the frame then, in
/// image pixels.
struct FrameDrag {
    grip: Option<Grip>,
    from: P,
    was: (P, P),
    /// Whether the pointer has moved far enough to be a drag; a click
    /// alone leaves the area as it was.
    moved: bool,
    /// A crop (from the crop tool, or the picture's own edge): applied as
    /// soon as it is let go.
    quick: bool,
}

/// How far, in window points, the pointer goes before a press is a drag.
const DRAG_SLOP: f32 = 3.;
/// How far in from the picture's edge, and out from it, a drag takes hold
/// of the edge to crop or add room, in window points.
const EDGE_IN: f32 = 3.;
const EDGE_OUT: f32 = 8.;

struct TextEdit {
    /// Top left, in image pixels.
    at: P,
    input: Entity<InputState>,
    _sub: Subscription,
}

impl Editor {
    /// The editor on `editable`; `pictures` are its images as GPUI shows
    /// them, made off the main thread (see [`Editor::pictures`]).
    fn new(entry: Entry, editable: Editable, pictures: HashMap<u32, Arc<RenderImage>>) -> Editor {
        let Editable { original, edits, images } = editable;
        let size = original.image.dimensions();
        let mut ed = Editor {
            entry,
            original: Arc::new(original.image),
            doc: Doc::new(original.scale),
            size,
            pictures,
            under: None,
            under_key: None,
            tool: None,
            style: Style::default(),
            drag: None,
            text: None,
            pick: None,
            area: None,
            pasted: None,
            frame_drag: None,
            hover: None,
            edits,
            images,
            changed: false,
            zoom: None,
            pan: P::default(),
            panning: None,
            layer: None,
            moving: None,
            over_layer: false,
            resize: None,
            select_after: false,
        };
        ed.refresh();
        ed
    }

    /// The images of `editable` as GPUI shows them: a big capture takes a
    /// while to turn, so this is done before the editor opens on it.
    fn pictures(editable: &Editable) -> HashMap<u32, Arc<RenderImage>> {
        let mut out = HashMap::new();
        out.insert(ORIGINAL, picture(&editable.original.image));
        for (n, img) in &editable.images {
            out.insert(*n, picture(img));
        }
        out
    }

    /// Do `op`, to be undone later.
    fn push(&mut self, op: Op) {
        self.edits.push(op);
        self.changed = true;
        self.refresh();
    }

    /// Undo the last step; false if there was none.
    fn undo(&mut self) -> bool {
        if self.edits.undo().is_none() {
            return false;
        }
        self.changed = true;
        self.refresh();
        true
    }

    fn redo(&mut self) {
        if self.edits.redo().is_some() {
            self.changed = true;
            self.refresh();
        }
    }

    /// Play the steps again, to show what they make now.
    fn refresh(&mut self) {
        let (doc, size) = edits::play(self.original.dimensions(), &self.edits.done, self.doc.scale);
        if size != self.size {
            self.zoom = None;
            self.pan = P::default();
        }
        self.doc = doc;
        self.size = size;
        for shape in &self.doc.shapes {
            if let Shape::Image { image, .. } = shape
                && !self.pictures.contains_key(image)
                && let Some(img) = self.pixels(*image)
            {
                let pic = picture(img);
                self.pictures.insert(*image, pic);
            }
        }
        let mosaic = self.doc.visible().any(|s| matches!(s, Shape::Mosaic { .. }));
        if mosaic {
            let key = (self.size, self.doc.visible().filter(|s| matches!(s, Shape::Image { .. })).cloned().collect());
            if self.under.is_none() || self.under_key.as_ref() != Some(&key) {
                self.under = Some(Arc::new(self.compose(true)));
                self.under_key = Some(key);
            }
        } else {
            self.under = None;
            self.under_key = None;
        }
        self.keep_layer();
    }

    /// Image `n`'s pixels: the capture as taken, or a pasted image.
    fn pixels(&self, n: u32) -> Option<&RgbaImage> {
        if n == ORIGINAL { Some(&self.original) } else { self.images.get(&n).map(|i| &**i) }
    }

    /// The canvas flattened: every layer shown, or only the images.
    fn compose(&self, images_only: bool) -> RgbaImage {
        let (w, h) = self.size;
        if !images_only {
            return raster::compose(w, h, &self.doc, |n| self.pixels(n), raster::font());
        }
        let mut doc = Doc::new(self.doc.scale);
        for (i, s) in self.doc.shapes.iter().enumerate() {
            if matches!(s, Shape::Image { .. }) && !self.doc.is_hidden(i) {
                doc.push(s.clone());
            }
        }
        raster::compose(w, h, &doc, |n| self.pixels(n), None)
    }

    /// Let go of the picked layer.
    fn unpick(&mut self) {
        self.layer = None;
        self.resize = None;
    }

    /// The picked layer's rectangle, if it is an image.
    fn picked_image(&self) -> Option<(P, P)> {
        self.doc.shapes.get(self.layer?)?.image_rect()
    }

    /// Lay the picked image into `rect`, on whole pixels.
    fn finish_resize(&mut self, (lo, hi): (P, P)) {
        let Some(i) = self.layer else { return };
        let Some(Shape::Image { image, .. }) = self.doc.shapes.get(i) else { return };
        let round = |q: P| p(q.x.round(), q.y.round());
        let (a, b) = (round(lo), round(hi));
        let b = p(b.x.max(a.x + 1.), b.y.max(a.y + 1.));
        let shape = Shape::Image { image: *image, a, b };
        if self.doc.shapes.get(i) != Some(&shape) {
            self.push(Op::Change { layer: i, shape });
        }
    }

    /// Let go of the picked layer if it is gone.
    fn keep_layer(&mut self) {
        self.moving = None;
        self.layer = self.layer.filter(|&i| i < self.doc.shapes.len());
    }

    /// The shown layer under `q` (canvas pixels): the topmost, unless the
    /// picked one is a shape there.
    fn layer_at(&self, q: P, reach: f32) -> Option<usize> {
        let font = raster::font();
        let picked = self.layer.filter(|&i| {
            let s = &self.doc.shapes[i];
            !matches!(s, Shape::Image { .. }) && !self.doc.is_hidden(i) && raster::hits(s, q, reach, self.doc.scale, font)
        });
        picked.or_else(|| raster::layer_at(&self.doc, q, reach, font))
    }

    /// Hide layer `i`, or show it again.
    fn toggle_hidden(&mut self, i: usize) {
        if i < self.doc.shapes.len() {
            let hidden = !self.doc.is_hidden(i);
            self.push(Op::Hide { layer: i, hidden });
        }
    }

    fn remove_layer(&mut self) {
        if let Some(i) = self.layer.take() {
            self.push(Op::Remove { layer: i });
        }
    }

    /// Move the picked layer `by` places up (towards the top), or down.
    fn reorder_layer(&mut self, by: isize) {
        let Some(i) = self.layer else { return };
        let Some(to) = i.checked_add_signed(by).filter(|&to| to < self.doc.shapes.len()) else { return };
        self.push(Op::Reorder { layer: i, to });
        self.layer = Some(to);
    }

    /// The style the style bar shows and changes: the picked layer's when
    /// no tool is, else the tool's. `None` when there is none to show.
    fn shown_style(&self) -> Option<Style> {
        match (self.tool, self.layer) {
            (Some(Tool::Mosaic), _) => None,
            (Some(_), _) => Some(self.style),
            (None, Some(i)) => self.doc.shapes.get(i)?.style(),
            (None, None) => None,
        }
    }

    /// Change the style the style bar shows.
    fn restyle(&mut self, f: impl FnOnce(&mut Style)) {
        if self.tool.is_none()
            && let Some(i) = self.layer
            && let Some(shape) = self.doc.shapes.get(i)
            && let Some(was) = shape.style()
        {
            let mut style = was;
            f(&mut style);
            if style != was {
                let shape = shape.with_style(style);
                self.push(Op::Change { layer: i, shape });
            }
            return;
        }
        f(&mut self.style);
    }

    fn can_redo(&self) -> bool {
        !self.edits.undone.is_empty()
    }

    /// What makes the capture with its markup, at full resolution (only
    /// the selected area of it, with `selection`), to run off the main
    /// thread: a big canvas takes a while.
    fn flattener(&self, selection: bool) -> impl FnOnce() -> RgbaImage + Send + 'static {
        let crop = if selection { self.selection_rect() } else { None };
        let (w, h) = self.size;
        let (doc, original, images) = (self.doc.clone(), self.original.clone(), self.images.clone());
        move || {
            let img = raster::compose(
                w,
                h,
                &doc,
                |n| if n == ORIGINAL { Some(&*original) } else { images.get(&n).map(|i| &**i) },
                raster::font(),
            );
            match crop {
                Some((x, y, cw, ch)) => image::imageops::crop_imm(&img, x, y, cw, ch).to_image(),
                None => img,
            }
        }
    }

    fn changed(&self) -> bool {
        self.changed
    }

    /// Crop the canvas to the area picked; where the area runs past it,
    /// blank room is added.
    fn apply_crop(&mut self) {
        self.frame_drag = None;
        let Some((lo, hi)) = self.area.take() else { return };
        self.pick = None;
        if (lo, hi) == self.whole() {
            return;
        }
        let (x, y) = (lo.x.round() as i64, lo.y.round() as i64);
        let (w, h) = ((hi.x.round() as i64 - x).max(1) as u32, (hi.y.round() as i64 - y).max(1) as u32);
        self.push(Op::Crop { x, y, w, h });
    }

    /// The selected area, in canvas pixels: x, y, width, height.
    fn selection_rect(&self) -> Option<(u32, u32, u32, u32)> {
        let (lo, hi) = self.area.filter(|_| self.pick == Some(Pick::Select))?;
        let (x, y) = (lo.x.round().max(0.) as u32, lo.y.round().max(0.) as u32);
        let (w, h) = ((hi.x.round() as u32).saturating_sub(x).max(1), (hi.y.round() as u32).saturating_sub(y).max(1));
        Some((x, y, w, h))
    }

    /// Delete what is in the selected area, as one step: the layers wholly
    /// inside it go, and where it cuts through one, that part is erased to
    /// blank paper.
    fn delete_area(&mut self) {
        let Some((lo, hi)) = self.area.filter(|_| self.pick == Some(Pick::Select)) else { return };
        let font = raster::font();
        let mut steps = Vec::new();
        let mut cuts = false;
        // From the top, so each place counted is still right when it goes.
        for i in (0..self.doc.shapes.len()).rev().filter(|&i| !self.doc.is_hidden(i)) {
            let (a, b) = raster::bounds(&self.doc.shapes[i], self.doc.scale, font);
            if a.x >= lo.x - 0.5 && a.y >= lo.y - 0.5 && b.x <= hi.x + 0.5 && b.y <= hi.y + 0.5 {
                steps.push(Op::Remove { layer: i });
            } else if a.x < hi.x && b.x > lo.x && a.y < hi.y && b.y > lo.y {
                cuts = true;
            }
        }
        if cuts {
            steps.push(Op::Shape { shape: Shape::Erase { a: lo, b: hi } });
        }
        match steps.len() {
            0 => {},
            1 => self.push(steps.remove(0)),
            _ => self.push(Op::Steps { steps }),
        }
        self.area = None;
        self.frame_drag = None;
    }

    /// Lay the pasted image down where it is, as a layer of its own; the
    /// canvas grows to hold it.
    fn commit_paste(&mut self) {
        let Some(ps) = self.pasted.take() else { return };
        self.frame_drag = None;
        let (lo, hi) = ps.rect;
        let (x, y) = (lo.x.round() as i64, lo.y.round() as i64);
        let (w, h) = ((hi.x.round() as i64 - x).max(1) as u32, (hi.y.round() as i64 - y).max(1) as u32);
        let image = self.images.keys().copied().chain(self.edits.images()).max().map_or(0, |n| n + 1);
        self.images.insert(image, ps.img);
        self.push(Op::Paste { image, x, y, w, h });
        // Picked, to move or resize again.
        self.layer = self.doc.shapes.len().checked_sub(1);
    }

    /// The frame shown with handles: the pasted image, else the picked
    /// image, else the area.
    fn frame(&self) -> Option<(P, P)> {
        match &self.pasted {
            Some(ps) => Some(ps.rect),
            None if self.resize.is_some() => self.resize,
            None if self.tool.is_none() && self.pick.is_none() && self.picked_image().is_some() => self.picked_image(),
            None => self.area.filter(|_| self.pick.is_some()),
        }
    }

    fn can_undo(&self) -> bool {
        !self.edits.done.is_empty()
    }

    /// The zoom that fits the canvas in an area `aw` × `ah` points, never
    /// past the capture's own size.
    fn fit(&self, aw: f32, ah: f32) -> f32 {
        let (iw, ih) = (self.size.0 as f32, self.size.1 as f32);
        (aw / iw).min(ah / ih).min(1. / self.doc.scale)
    }

    /// The whole canvas, as an area.
    fn whole(&self) -> (P, P) {
        (P::default(), p(self.size.0 as f32, self.size.1 as f32))
    }

    fn clamp_to_image(&self, q: P) -> P {
        p(q.x.clamp(0., self.size.0 as f32), q.y.clamp(0., self.size.1 as f32))
    }
}

impl Library {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // The capture toolbar is away while the library is open, and
        // Gannet is in the Dock.
        toolbar::library_open(true, cx);
        dock::set(dock::Window::Library, true, cx);
        let me = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            // What the editor has open is kept.
            let _ = me.update(cx, |this, cx| {
                if let Some(ed) = this.editor.take() {
                    this.keep_changes(ed, cx);
                }
            });
            toolbar::library_open(false, cx);
            dock::set(dock::Window::Library, false, cx);
            true
        });
        cx.on_release(|_, cx| {
            toolbar::library_open(false, cx);
            dock::set(dock::Window::Library, false, cx)
        })
        .detach();
        let mut this = Self {
            focus,
            entries: Vec::new(),
            thumbs: HashMap::new(),
            asked: HashSet::new(),
            wanted: Default::default(),
            fetching: false,
            frame: 0,
            error: None,
            editor: None,
            opening: None,
            selected: Vec::new(),
            anchor: None,
            side_rows: Default::default(),
            side_list: ListState::new(0, ListAlignment::Top, px(200.)),
            grid_rows: Default::default(),
            grid_list: ListState::new(0, ListAlignment::Top, px(300.)),
            side_key: Default::default(),
            grid_key: Default::default(),
            listed: 0,
            window_width: std::cell::Cell::new(1200.),
            title: TITLE.to_owned(),
            expanded: false,
            loaded: false,
            reveal: std::cell::Cell::new(false),
            save_soon: None,
            layers_shown: prefs::get(cx).layers,
            folders: Folders::default(),
            folders_loaded: false,
            folded: HashSet::new(),
            renaming: None,
            trashed: Vec::new(),
            trash_open: false,
        };
        this.reload(cx);
        this
    }

    /// List the captures again and fetch the thumbnails not yet shown.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let read_folders = !self.folders_loaded;
        cx.spawn(async move |this, cx| {
            let listed = cx
                .background_executor()
                .spawn(async move { vault().map(|v| (v.list(), read_folders.then(|| v.folders()), v.trash())) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match listed {
                    Ok((entries, folders, trash)) => {
                        this.error = None;
                        let trash = trash.unwrap_or_else(|e| {
                            log::warn!("{e}");
                            Default::default()
                        });
                        // What is in the trash is listed there alone, latest first.
                        let in_trash: HashSet<&str> = trash.items.iter().map(|t| t.id.as_str()).collect();
                        let (mut trashed, live): (Vec<Entry>, Vec<Entry>) =
                            entries.into_iter().partition(|e| in_trash.contains(e.id.as_str()));
                        let order: HashMap<&str, usize> = trash.items.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
                        trashed.sort_by_key(|e| order.get(e.id.as_str()).copied().unwrap_or(usize::MAX));
                        this.trashed = trashed;
                        this.entries = live;
                        this.listed += 1;
                        match folders {
                            Some(Ok(f)) => {
                                this.folders = f;
                                this.folders_loaded = true;
                            },
                            Some(Err(e)) => {
                                log::warn!("{e}");
                                this.folders_loaded = true;
                            },
                            None => {},
                        }
                        let ids: HashSet<&str> = this.entries.iter().map(|e| e.id.as_str()).collect();
                        this.selected.retain(|id| ids.contains(id.as_str()));
                        let trashed: HashSet<&str> = this.trashed.iter().map(|e| e.id.as_str()).collect();
                        let listed = |(id, _): &ThumbKey| ids.contains(id.as_str()) || trashed.contains(id.as_str());
                        this.thumbs.retain(|key, _| listed(key));
                        this.asked.retain(listed);
                        let mut changed = this.folders.keep_only(&ids);
                        changed |= this.file_loose();
                        if changed {
                            this.save_folders(cx);
                        }
                    },
                    Err(e) => this.error = Some(e),
                }
                this.loaded = true;
                // The editor is the library's main view: with nothing open,
                // the newest capture is.
                if this.editor.is_none()
                    && this.opening.is_none()
                    && let Some(newest) = this.entries.first().cloned()
                {
                    this.open_capture(newest, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---- Folders ----

    /// Put each capture in no folder in its day's, the oldest first so the
    /// newest day's folder ends up on top. Whether any was.
    fn file_loose(&mut self) -> bool {
        self.folders.file_all(self.entries.iter().rev().map(|e| (e.id.as_str(), day_folder(e))))
    }

    /// The captures by folder, as listed: the folder, its name, and its
    /// captures, newest first.
    fn groups(&self) -> Vec<(u64, String, Vec<Entry>)> {
        let mut out: Vec<(u64, String, Vec<Entry>)> = self.folders.list.iter().map(|f| (f.id, f.name.clone(), Vec::new())).collect();
        let mut loose = Vec::new();
        let folder_of = self.folders.index();
        let place: HashMap<u64, usize> = out.iter().enumerate().map(|(i, g)| (g.0, i)).collect();
        for e in &self.entries {
            match folder_of.get(e.id.as_str()).and_then(|id| place.get(id).copied()) {
                Some(i) => out[i].2.push(e.clone()),
                None => loose.push(e.clone()),
            }
        }
        if !loose.is_empty() {
            out.push((LOOSE, "Unsorted".to_owned(), loose));
        }
        out
    }

    fn save_folders(&mut self, cx: &mut Context<Self>) {
        let folders = self.folders.clone();
        cx.spawn(async move |_, cx| {
            let saved = cx.background_executor().spawn(async move { vault()?.save_folders(&folders) }).await;
            if let Err(e) = saved {
                cx.update(|cx| capture::notify("Couldn't keep the folders", &e, cx));
            }
        })
        .detach();
    }

    /// Move captures `ids` into folder `to`.
    fn move_to(&mut self, ids: &[String], to: u64, cx: &mut Context<Self>) {
        if to == LOOSE {
            return;
        }
        let mut changed = false;
        for id in ids {
            changed |= self.folders.put(id, Some(to));
        }
        if changed {
            self.folded.remove(&to);
            self.save_folders(cx);
        }
        cx.notify();
    }

    /// A new folder on top, its name being typed.
    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.folders.add(&self.folders.new_name());
        self.save_folders(cx);
        self.start_rename(id, window, cx);
    }

    /// Delete folder `id`; the captures in it go to the trash, to come
    /// back to it.
    fn remove_folder(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ids) = self.folders.get(id).map(|f| f.captures.clone()) else { return };
        // Taken to the trash with their folder's name, then the folder goes.
        self.delete(ids, window, cx);
        self.folders.remove(id);
        self.folded.remove(&id);
        self.save_folders(cx);
        cx.notify();
    }

    fn start_rename(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.folders.get(id).map(|f| f.name.clone()) else { return };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, _, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.finish_rename(cx),
            _ => {},
        });
        let handle = input.focus_handle(cx);
        let select = input.clone();
        window.defer(cx, move |window, cx| {
            handle.focus(window, cx);
            select.update(cx, |s, cx| s.select_all(window, cx));
        });
        self.renaming = Some(Renaming { id, input, _sub: sub });
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.renaming.take() else { return };
        let name = r.input.read(cx).value().to_string();
        if self.folders.rename(r.id, &name) {
            self.save_folders(cx);
        }
        cx.notify();
    }

    /// The captures a drag from capture `id` takes: those picked, if it is
    /// one of them, else it alone.
    fn dragged(&self, id: &str, label: String) -> Dragged {
        let ids = if self.selected.iter().any(|s| s == id) { self.selected.clone() } else { vec![id.to_owned()] };
        let label = if ids.len() > 1 { count(ids.len()) } else { label };
        Dragged { ids, label: label.into() }
    }

    /// A folder's title, in the side list or the grid: a drop target for
    /// captures, its name renamed by a double click; in the tree, a click
    /// opens or closes it.
    fn folder_header(&self, at: &str, id: u64, name: &str, entries: &[Entry], tree: bool, cx: &mut Context<Self>) -> AnyElement {
        let folded = tree && self.folded.contains(&id);
        let pick = self.day_pick(at, &format!("{id}"), entries, cx);
        let title: AnyElement = match &self.renaming {
            Some(r) if r.id == id => div()
                .flex_1()
                .min_w_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(&r.input).appearance(false).text_size(px(12.)))
                .into_any_element(),
            _ => div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(name.to_owned())
                .into_any_element(),
        };
        let real = id != LOOSE;
        let picking = !self.selected.is_empty();
        div()
            .id(SharedString::from(format!("{at}-folder-{id}")))
            .group("day")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(30.))
            .pl(px(4.))
            .pr(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff0d)))
            .when(real, |d| d.drag_over::<Dragged>(|s, _, _, _| s.bg(rgba(0xffffff26))))
            .when(real, |d| d.on_drop(cx.listener(move |this, dragged: &Dragged, _, cx| this.move_to(&dragged.ids, id, cx))))
            .when(tree, |d| {
                d.child(div().text_color(rgb(DIM)).child(icon(if folded { IconName::ChevronRight } else { IconName::ChevronDown }, 14.)))
            })
            .child(div().text_color(rgb(DIM)).child(icon(if folded || !tree { IconName::Folder } else { IconName::FolderOpen }, 14.)))
            .child(title)
            // At the right edge, the count; in its place under the pointer,
            // or while picking, what can be done to the folder.
            .child(
                div()
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .min_w(px(18.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(DIM))
                            .map(|d| if picking { d.opacity(0.) } else { d.group_hover("day", |s| s.opacity(0.)) })
                            .child(entries.len().to_string()),
                    )
                    .child(
                        div()
                            .absolute()
                            .right_0()
                            .top_0()
                            .bottom_0()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .map(|d| if picking { d } else { d.opacity(0.).group_hover("day", |s| s.opacity(1.)) })
                            .when(real, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("{at}-folder-x-{id}")))
                                        .text_color(rgb(DIM))
                                        .hover(|s| s.text_color(rgb(TEXT)))
                                        .tooltip(tip("Delete the folder (its captures go to the trash)"))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                            cx.stop_propagation();
                                            this.remove_folder(id, window, cx);
                                        }))
                                        .child(icon(IconName::Trash, 13.)),
                                )
                            })
                            .child(pick),
                    ),
            )
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                if real && ev.click_count() >= 2 {
                    return this.start_rename(id, window, cx);
                }
                if tree && !this.folded.remove(&id) {
                    this.folded.insert(id);
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// Thumbnail `id`, drawn to fit `w` × `h`; until it is read, a blank
    /// `blank` points square, and it is asked for. A small place (a side
    /// list row) gets a small thumbnail, a card the whole one.
    fn thumb_element(&self, id: &str, w: f32, h: f32, blank: f32) -> AnyElement {
        let key = (id.to_owned(), w > ROW_PIC_W || h > ROW_PIC_H);
        match self.thumbs.get(&key) {
            Some(t) => {
                t.shown.set(self.frame);
                let (tw, th) = t.size;
                let k = (w / tw as f32).min(h / th as f32);
                img(ImageSource::Render(t.picture.clone())).w(px(tw as f32 * k)).h(px(th as f32 * k)).into_any_element()
            },
            None => {
                if !self.asked.contains(&key) {
                    self.wanted.borrow_mut().push(key);
                }
                div().size(px(blank)).into_any_element()
            },
        }
    }

    /// Keep thumbnail `img` of capture `id` (a fresh one, after an edit),
    /// in both sizes.
    fn keep_thumb(&mut self, id: String, img: RgbaImage) {
        for big in [false, true] {
            let (picture, size) = shown_thumb(img.clone(), big);
            let key = (id.clone(), big);
            self.asked.insert(key.clone());
            self.thumbs.insert(key, Thumb { picture, size, shown: std::cell::Cell::new(self.frame) });
        }
        self.forget_old_thumbs();
    }

    /// Past [`THUMBS_KEPT`], let go of the thumbnails shown longest ago;
    /// they are read again if they come back into sight.
    fn forget_old_thumbs(&mut self) {
        if self.thumbs.len() <= THUMBS_KEPT {
            return;
        }
        let mut by_age: Vec<(u64, ThumbKey)> = self.thumbs.iter().map(|(key, t)| (t.shown.get(), key.clone())).collect();
        by_age.sort_unstable();
        // Down well under the limit, so this doesn't run on every one read.
        let excess = self.thumbs.len() - THUMBS_KEPT * 3 / 4;
        for (shown, key) in by_age.into_iter().take(excess) {
            if shown + 1 >= self.frame {
                break; // in sight now
            }
            self.thumbs.remove(&key);
            self.asked.remove(&key);
        }
    }

    /// Read the thumbnails rows in sight want, a batch at a time, decoded
    /// off the main thread, each batch shown at once.
    fn fetch_wanted(&mut self, cx: &mut Context<Self>) {
        if self.fetching || self.wanted.borrow().is_empty() {
            return;
        }
        self.fetching = true;
        // Run after this frame, so every row in sight has said what it wants.
        cx.spawn(async move |this, cx| {
            loop {
                let ids = this.update(cx, |this, _| {
                    let ids: Vec<ThumbKey> = this.wanted.borrow_mut().drain(..).filter(|key| this.asked.insert(key.clone())).collect();
                    if ids.is_empty() {
                        this.fetching = false;
                    }
                    ids
                });
                let Ok(ids) = ids else { return };
                if ids.is_empty() {
                    return;
                }
                for batch in ids.chunks(THUMB_BATCH) {
                    let batch = batch.to_vec();
                    let read = cx
                        .background_executor()
                        .spawn(async move {
                            let v = vault();
                            batch
                                .into_iter()
                                .map(|key| {
                                    let t = v.as_ref().map_err(Clone::clone).and_then(|v| v.thumb(&key.0));
                                    let big = key.1;
                                    (key, t.map(|t| shown_thumb(t.image, big)))
                                })
                                .collect::<Vec<_>>()
                        })
                        .await;
                    let ok = this.update(cx, |this, cx| {
                        for (key, t) in read {
                            match t {
                                Ok((picture, size)) => {
                                    this.thumbs.insert(key, Thumb { picture, size, shown: std::cell::Cell::new(this.frame) });
                                },
                                Err(e) => log::warn!("thumbnail {}: {e}", key.0),
                            }
                        }
                        this.forget_old_thumbs();
                        cx.notify();
                    });
                    if ok.is_err() {
                        return;
                    }
                }
            }
        })
        .detach();
    }

    // ---- The grid ----

    fn click_card(&mut self, i: usize, m: Modifiers, cx: &mut Context<Self>) {
        let Some(e) = self.entries.get(i).cloned() else { return };
        if m.shift || m.secondary() {
            return self.toggle_card(i, m.shift, cx);
        }
        self.anchor = Some(i);
        self.expanded = false;
        self.open_capture(e, cx);
    }

    /// Pick card `i`, or let it go; with `range`, pick every card from the
    /// last one clicked to `i`.
    fn toggle_card(&mut self, i: usize, range: bool, cx: &mut Context<Self>) {
        let Some(e) = self.entries.get(i) else { return };
        if range {
            let from = self.anchor.unwrap_or(i);
            let (a, b) = (from.min(i), from.max(i));
            for e in &self.entries[a..=b] {
                if !self.selected.contains(&e.id) {
                    self.selected.push(e.id.clone());
                }
            }
        } else {
            match self.selected.iter().position(|id| *id == e.id) {
                Some(at) => {
                    self.selected.remove(at);
                },
                None => self.selected.push(e.id.clone()),
            }
            self.anchor = Some(i);
        }
        cx.notify();
    }

    fn picked(&self) -> Vec<Entry> {
        self.entries.iter().filter(|e| self.selected.contains(&e.id)).cloned().collect()
    }

    /// Write the picked captures, as they are, to a folder the user picks.
    fn export_picked(&mut self, cx: &mut Context<Self>) {
        let picked = self.picked();
        if picked.is_empty() {
            return;
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(format!("Export {}", count(picked.len())).into()),
        });
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(dir) = paths.into_iter().next() else { return };
            let done = cx
                .background_executor()
                .spawn(async move {
                    let v = vault()?;
                    for e in &picked {
                        let img = v.load(&e.id)?.image;
                        img.save_with_format(vault::export_path(&dir, e), image::ImageFormat::Png).map_err(|e| e.to_string())?;
                    }
                    let _ = std::process::Command::new("/usr/bin/open").arg(&dir).spawn();
                    Ok::<_, String>(())
                })
                .await;
            if let Err(e) = done {
                cx.update(|cx| capture::notify("Couldn't export the captures", &e, cx));
            }
        })
        .detach();
    }

    /// Put the picked captures on the clipboard. One goes as an image; several
    /// go as PNG files in a private temporary folder, since apps paste only
    /// the first of several images but take every file of a list.
    fn copy_picked(&mut self, cx: &mut Context<Self>) {
        let picked = self.picked();
        self.copy_entries(picked, cx);
    }

    /// Copy `picked`: one as an image, several as PNG files.
    fn copy_entries(&mut self, picked: Vec<Entry>, cx: &mut Context<Self>) {
        if picked.is_empty() {
            return;
        }
        cx.spawn(async move |_, cx| {
            let copied = cx
                .background_executor()
                .spawn(async move {
                    let v = vault()?;
                    if let [e] = picked.as_slice() {
                        return store::png_bytes(&v.load(&e.id)?.image).map(Copied::Image);
                    }
                    // The files of the last copy go: only one set lies around.
                    let dir = copied_dir();
                    forget_copied();
                    store::private_dir(&dir)?;
                    let mut paths = Vec::new();
                    for e in &picked {
                        let path = vault::export_path(&dir, e);
                        v.load(&e.id)?.image.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
                        paths.push(path);
                    }
                    Ok::<_, String>(Copied::Files(paths))
                })
                .await;
            cx.update(|cx| match copied {
                Ok(Copied::Image(png)) => cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, png))),
                Ok(Copied::Files(paths)) => {
                    copy_files(&paths);
                    forget_copied_later(cx);
                },
                Err(e) => capture::notify("Couldn't copy the captures", &e, cx),
            });
        })
        .detach();
    }

    /// Put captures `ids` in the trash, each with the folder it was in.
    fn delete(&mut self, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let items: Vec<(String, Option<String>)> = ids
            .iter()
            .map(|id| (id.clone(), self.folders.folder_of(id).and_then(|f| self.folders.get(f)).map(|f| f.name.clone())))
            .collect();
        for id in &ids {
            self.folders.put(id, None);
        }
        self.save_folders(cx);
        let now = chrono::Utc::now().timestamp_millis();
        self.in_vault(move |v| v.to_trash(&items, now), "Couldn't move to the trash", ids, window, cx);
    }

    /// Delete captures `ids`, in the trash, for good.
    fn delete_for_good(&mut self, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let gone = ids.clone();
        self.in_vault(
            move |v| {
                // Copies of them as files would outlive them.
                forget_copied();
                v.delete_for_good(&ids)
            },
            "Couldn't delete the captures",
            gone,
            window,
            cx,
        );
    }

    /// Delete everything in the trash (`ids`) for good, once the user agrees.
    fn empty_trash(&mut self, ids: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let title = "Empty the trash?";
        let detail = format!(
            "{} will be deleted for good. This can't be undone.",
            if ids.len() == 1 { "The capture in it".to_owned() } else { format!("The {} in it", count(ids.len())) }
        );
        let answer = window.prompt(PromptLevel::Warning, title, Some(&detail), &["Empty Trash", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update_in(cx, |this, window, cx| this.delete_for_good(ids, window, cx));
            }
        })
        .detach();
    }

    /// Take captures `ids` out of the trash, back to the folders they were in.
    fn restore(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let back = cx.background_executor().spawn(async move { vault()?.restore(&ids) }).await;
            let _ = this.update(cx, |this, cx| {
                match back {
                    Ok(items) => {
                        for t in items {
                            if let Some(name) = t.folder {
                                this.folders.file(&t.id, &name);
                            }
                        }
                        this.save_folders(cx);
                    },
                    Err(e) => capture::notify("Couldn't put the captures back", &e, cx),
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Do `f` to the library in the background, then list it again; the
    /// captures `gone` leave the editor and the picks.
    fn in_vault(
        &mut self,
        f: impl FnOnce(&Vault) -> Result<(), String> + Send + 'static,
        failed: &'static str,
        gone: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.as_ref().is_some_and(|ed| gone.contains(&ed.entry.id))
            && let Some(ed) = self.editor.take()
        {
            self.keep_changes(ed, cx);
            window.focus(&self.focus, cx);
        }
        self.selected.retain(|id| !gone.contains(id));
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { f(&*vault()?) }).await;
            let _ = this.update(cx, |this, cx| {
                if let Err(e) = done {
                    capture::notify(failed, &e, cx);
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    // ---- The editor ----

    fn open_capture(&mut self, entry: Entry, cx: &mut Context<Self>) {
        self.opening = Some(entry.id.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let id = entry.id.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let editable = {
                        let _writes = VAULT_WRITES.lock();
                        vault()?.load_editable(&id)?
                    };
                    let pictures = Editor::pictures(&editable);
                    Ok::<_, String>((editable, pictures))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.opening.as_deref() != Some(&entry.id) {
                    return;
                }
                this.opening = None;
                match loaded {
                    Ok((editable, pictures)) => {
                        // The capture it replaces stayed on screen until now.
                        if let Some(old) = this.editor.take() {
                            this.keep_changes(old, cx);
                        }
                        this.editor = Some(Editor::new(entry, editable, pictures));
                        this.reveal.set(true);
                    },
                    Err(e) => capture::notify("Couldn't open the capture", &e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Show every capture over the editor, or go back to it.
    fn expand(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        self.expanded = on;
        cx.notify();
    }

    /// Save what was drawn on `ed`, if anything.
    fn keep_changes(&mut self, mut ed: Editor, cx: &mut Context<Self>) {
        ed.commit_paste();
        self.save(&mut ed, cx);
    }

    /// Save the open capture's changes a moment from now, once they stop.
    fn save_soon(&mut self, cx: &mut Context<Self>) {
        if self.save_soon.is_some() || !self.editor.as_ref().is_some_and(Editor::changed) {
            return;
        }
        self.save_soon = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                this.save_soon = None;
                if let Some(mut ed) = this.editor.take() {
                    this.save(&mut ed, cx);
                    this.editor = Some(ed);
                }
            });
        }));
    }

    /// Keep what was done to `ed` in the library, if anything was since the
    /// last save: the steps, and the capture as they make it.
    fn save(&mut self, ed: &mut Editor, cx: &mut Context<Self>) {
        if !ed.changed() {
            return;
        }
        ed.changed = false;
        let (id, scale) = (ed.entry.id.clone(), ed.doc.scale);
        let original = ed.original.clone();
        let (edits, images) = (ed.edits.clone(), ed.images.clone());
        let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        cx.spawn(async move |this, cx| {
            let key = id.clone();
            let saved = cx
                .background_executor()
                .spawn(async move {
                    let img = edits::render(&original, &edits.done, scale, &images, raster::font());
                    let thumb = vault::thumbnail(&img);
                    // Encoded before the files are held: opening another
                    // capture waits only for the writing.
                    let png = store::png_bytes(&img)?;
                    let mut writes = VAULT_WRITES.lock().unwrap_or_else(|e| e.into_inner());
                    let latest = writes.get_or_insert_default();
                    if latest.get(&id).is_some_and(|&m| m > n) {
                        return Ok(None);
                    }
                    latest.insert(id.clone(), n);
                    vault()?.save_edits(&id, &edits, &images, &img, Some(&png), scale)?;
                    Ok::<_, String>(Some(thumb))
                })
                .await;
            match saved {
                Ok(Some(thumb)) => {
                    let _ = this.update(cx, |this, cx| {
                        this.keep_thumb(key, thumb);
                        cx.notify();
                    });
                },
                Ok(None) => {},
                Err(e) => cx.update(|cx| capture::notify("Couldn't keep the changes", &e, cx)),
            }
        })
        .detach();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.editor {
            ed.undo();
        }
        cx.notify();
    }

    /// Open `entry` in the editor instead of the open capture, which stays
    /// on screen until `entry` is loaded (no flash of the grid between).
    fn switch_to(&mut self, entry: Entry, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.as_ref().is_some_and(|ed| ed.entry.id == entry.id) {
            return;
        }
        self.commit_text(window, cx);
        self.open_capture(entry, cx);
    }

    /// The capture `by` places after the open one (before, if negative).
    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &self.editor else { return };
        let Some(at) = self.entries.iter().position(|e| e.id == ed.entry.id) else { return };
        let Some(next) = at.checked_add_signed(by).and_then(|i| self.entries.get(i)).cloned() else { return };
        self.switch_to(next, window, cx);
    }

    /// Copy the selected area, or the whole capture when none is.
    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        let Some(ed) = &mut self.editor else { return };
        ed.commit_paste();
        let make = ed.flattener(true);
        cx.spawn(async move |_, cx| {
            let png = cx.background_executor().spawn(async move { store::png_bytes(&make()) }).await;
            cx.update(|cx| match png {
                Ok(bytes) => cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, bytes))),
                Err(e) => capture::notify("Couldn't copy the capture", &e, cx),
            });
        })
        .detach();
        cx.notify();
    }

    /// Paste the clipboard's image over the capture, in the middle of what
    /// is shown, to be moved and resized before it is laid down.
    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        let Some(img) = clipboard_image(cx) else {
            return capture::notify("Nothing to paste", "The clipboard has no image.", cx);
        };
        let centre = self.to_px(self.area(window).2, window);
        let Some(ed) = &mut self.editor else { return };
        ed.commit_paste();
        ed.pick = None;
        ed.area = None;
        ed.tool = None;
        ed.unpick();
        let (w, h) = (img.width() as f32, img.height() as f32);
        let (bw, bh) = (ed.size.0 as f32, ed.size.1 as f32);
        let rect = if w <= bw && h <= bh {
            let c = centre.unwrap_or(p(bw / 2., bh / 2.));
            model::keep_inside((c - p(w / 2., h / 2.), c + p(w / 2., h / 2.)), bw, bh, true)
        } else {
            (P::default(), p(w, h))
        };
        ed.pasted = Some(Pasted { picture: picture(&img), img: Arc::new(img), rect });
        cx.notify();
    }

    /// Write the capture, marked up, to a file the user picks.
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        let Some(ed) = &mut self.editor else { return };
        ed.commit_paste();
        let dir = prefs::captures_dir(cx);
        let _ = std::fs::create_dir_all(&dir);
        let name = vault::export_path(&dir, &ed.entry).file_name().map(|n| n.to_string_lossy().into_owned());
        let make = ed.flattener(false);
        let rx = cx.prompt_for_new_path(&dir, name.as_deref());
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let written = cx
                .background_executor()
                .spawn(async move { make().save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string()) })
                .await;
            if let Err(e) = written {
                cx.update(|cx| capture::notify("Couldn't export the capture", &e, cx));
            }
        })
        .detach();
    }

    /// The room the list of captures takes on the stage's left.
    fn left(&self) -> f32 {
        SIDE + 2. * FLOAT
    }

    /// The room the layers take on the stage's right.
    fn right(&self) -> f32 {
        if self.layers_shown && !self.viewing_trash() { LAYERS + 2. * FLOAT } else { 0. }
    }

    /// Show the layers, or put them away.
    fn toggle_layers(&mut self, cx: &mut Context<Self>) {
        self.layers_shown = !self.layers_shown;
        let on = self.layers_shown;
        prefs::remember(cx, |s| s.layers = on);
        cx.notify();
    }

    /// Pick layer `i`, or none, putting the tools down.
    fn pick_layer(&mut self, i: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if let Some(ed) = &mut self.editor {
            ed.commit_paste();
            ed.tool = None;
            ed.pick = None;
            ed.area = None;
            ed.unpick();
            ed.layer = i.filter(|&i| i < ed.doc.shapes.len());
        }
        cx.notify();
    }

    /// Do `f` to the open capture's editor, and show it.
    fn with_editor(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Editor)) {
        if let Some(ed) = &mut self.editor {
            f(ed);
        }
        cx.notify();
    }

    /// The area the picture is shown in: (width, height, its centre).
    fn area(&self, window: &Window) -> (f32, f32, P) {
        let vp = window.viewport_size();
        let (w, h) = (vp.width.as_f32(), vp.height.as_f32());
        let w = w - self.left() - self.right();
        let (aw, ah) = ((w - 2. * PAD).max(1.), (h - TOOLBAR - 2. * PAD).max(1.));
        (aw, ah, p(self.left() + w / 2., TOOLBAR + PAD + ah / 2.))
    }

    /// Where the open capture lies in the window.
    fn view(&self, window: &Window) -> Option<View> {
        let ed = self.editor.as_ref()?;
        let (aw, ah, centre) = self.area(window);
        let k = ed.zoom.unwrap_or_else(|| ed.fit(aw, ah));
        let (iw, ih) = (ed.size.0 as f32 * k, ed.size.1 as f32 * k);
        let pan = if ed.zoom.is_some() { ed.pan } else { P::default() };
        Some(View { origin: centre - p(iw / 2., ih / 2.) + pan, k, scale: ed.doc.scale })
    }

    /// Zoom to `k`, keeping what is under window point `at` there.
    fn zoom_to(&mut self, k: f32, at: P, window: &Window, cx: &mut Context<Self>) {
        let Some(v0) = self.view(window) else { return };
        let (_, _, centre) = self.area(window);
        let Some(ed) = &mut self.editor else { return };
        let k = k.clamp(MIN_K, MAX_K);
        let under = (at - v0.origin).scale(1. / v0.k);
        let (iw, ih) = (ed.size.0 as f32 * k, ed.size.1 as f32 * k);
        let centred = centre - p(iw / 2., ih / 2.);
        ed.pan = at - under.scale(k) - centred;
        ed.zoom = Some(k);
        cx.notify();
    }

    fn zoom_by(&mut self, factor: f32, at: Option<P>, window: &Window, cx: &mut Context<Self>) {
        let Some(v) = self.view(window) else { return };
        let at = at.unwrap_or(self.area(window).2);
        self.zoom_to(v.k * factor, at, window, cx);
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        if let Some(ed) = &mut self.editor {
            ed.zoom = None;
            ed.pan = P::default();
        }
        cx.notify();
    }

    fn actual_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(scale) = self.editor.as_ref().map(|ed| ed.doc.scale) else { return };
        self.zoom_to(1. / scale, self.area(window).2, window, cx);
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(16.));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        if ev.modifiers.secondary() || ev.modifiers.control {
            return self.zoom_by((dy * 0.01).exp(), Some(from_point(ev.position)), window, cx);
        }
        let Some(v) = self.view(window) else { return };
        let Some(ed) = &mut self.editor else { return };
        if ed.zoom.is_none() {
            ed.zoom = Some(v.k);
        }
        ed.pan = ed.pan + p(dx, dy);
        cx.notify();
    }

    fn on_pinch(&mut self, ev: &PinchEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1. + ev.delta, Some(from_point(ev.position)), window, cx);
    }

    fn to_px(&self, q: P, window: &Window) -> Option<P> {
        let v = self.view(window)?;
        Some((q - v.origin).scale(1. / v.k))
    }

    fn on_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.as_ref().is_some_and(|ed| ed.text.is_some()) {
            return self.commit_text(window, cx);
        }
        let q = from_point(ev.position);
        let Some(at) = self.to_px(q, window) else { return };
        let Some(k) = self.view(window).map(|v| v.k) else { return };
        // A capture in the trash is only looked at: a drag moves the view.
        if self.viewing_trash() {
            if let Some(ed) = &mut self.editor {
                ed.panning = Some((q, if ed.zoom.is_some() { ed.pan } else { P::default() }));
            }
            return;
        }
        let Some(ed) = &mut self.editor else { return };
        let reach = GRIP_REACH / k;
        // Picked layers are let go of as soon as anything else is done.
        if ed.pasted.is_some() || ed.pick.is_some() || ed.tool.is_some() {
            ed.unpick();
        }
        if let Some(ps) = &ed.pasted {
            let (lo, hi) = ps.rect;
            match model::grip_at(lo, hi, at, reach) {
                Some(Grip::Move) if ev.click_count >= 2 => ed.commit_paste(),
                Some(grip) => ed.frame_drag = Some(FrameDrag { grip: Some(grip), from: at, was: ps.rect, moved: false, quick: false }),
                // A press away from it lays it down.
                None => ed.commit_paste(),
            }
            return cx.notify();
        }
        if ed.pick == Some(Pick::Crop) {
            // Drawn and cropped to at once; ⌘Z brings the rest back.
            ed.area = None;
            ed.frame_drag = Some(FrameDrag { grip: None, from: at, was: (at, at), moved: false, quick: true });
            return cx.notify();
        }
        let edge = model::edge_grip(P::default(), ed.whole().1, at, EDGE_IN / k, EDGE_OUT / k);
        if ed.pick == Some(Pick::Select) {
            let grip = ed.area.and_then(|(lo, hi)| model::grip_at(lo, hi, at, reach));
            // The canvas's edge still crops, the selection's handles aside.
            if grip.is_none()
                && let Some(edge) = edge
            {
                ed.select_after = true;
                ed.pick = Some(Pick::Crop);
                ed.area = Some(ed.whole());
                ed.zoom = Some(k);
                ed.frame_drag = Some(FrameDrag { grip: Some(edge), from: at, was: ed.whole(), moved: false, quick: true });
                return cx.notify();
            }
            let at = ed.clamp_to_image(at);
            ed.frame_drag = Some(FrameDrag { grip, from: at, was: ed.area.unwrap_or((at, at)), moved: false, quick: false });
            return cx.notify();
        }
        // No tool: a picked image's handles resize it, and a drag inside
        // it moves it.
        if ed.tool.is_none()
            && let Some((lo, hi)) = ed.picked_image()
            && let Some(grip) = model::grip_at(lo, hi, at, reach)
        {
            let above = ed.layer_at(at, reach).filter(|&i| Some(i) > ed.layer);
            if grip != Grip::Move {
                ed.zoom = Some(k);
                ed.resize = Some((lo, hi));
                ed.frame_drag = Some(FrameDrag { grip: Some(grip), from: at, was: (lo, hi), moved: false, quick: false });
                return cx.notify();
            }
            if above.is_none()
                && let Some(i) = ed.layer
            {
                ed.moving = Some(Moving { from: at, was: ed.doc.shapes[i].clone(), moved: false });
                return cx.notify();
            }
        }
        // No tool: the layer under the pointer is picked. A shape moves with
        // the drag at once; an image only once picked, so dragging the
        // picture still moves the view.
        // The canvas's edge goes before an image under it, which would
        // otherwise always be there when the picture fills the canvas.
        let hit = ed.layer_at(at, reach).filter(|&i| edge.is_none() || !matches!(ed.doc.shapes[i], Shape::Image { .. }));
        if ed.tool.is_none()
            && let Some(i) = hit
        {
            ed.unpick();
            ed.layer = Some(i);
            if matches!(ed.doc.shapes[i], Shape::Image { .. }) {
                ed.panning = Some((q, if ed.zoom.is_some() { ed.pan } else { P::default() }));
            } else {
                ed.moving = Some(Moving { from: at, was: ed.doc.shapes[i].clone(), moved: false });
            }
            return cx.notify();
        }
        // The picture's edge: drag it out to add room, in to crop.
        if let Some(grip) = edge {
            ed.pick = Some(Pick::Crop);
            ed.area = Some(ed.whole());
            // Keep the picture where it is while the area changes.
            ed.zoom = Some(k);
            ed.frame_drag = Some(FrameDrag { grip: Some(grip), from: at, was: ed.whole(), moved: false, quick: true });
            return cx.notify();
        }
        let Some(tool) = ed.tool else {
            // No tool, and no layer there: drag the view around.
            ed.unpick();
            ed.panning = Some((q, if ed.zoom.is_some() { ed.pan } else { P::default() }));
            return cx.notify();
        };
        let style = ed.style;
        ed.drag = Some(match tool {
            Tool::Rect => Shape::Rect { a: at, b: at, style },
            Tool::Ellipse => Shape::Ellipse { a: at, b: at, style },
            Tool::Line => Shape::Line { a: at, b: at, style },
            Tool::Arrow => Shape::Arrow { a: at, b: at, style },
            Tool::Pen => Shape::Pen { points: vec![at], style },
            Tool::Mosaic => Shape::Mosaic { a: at, b: at },
            Tool::Marker => {
                let n = ed.doc.next_marker();
                ed.push(Op::Shape { shape: Shape::Marker { at, n, style } });
                return cx.notify();
            },
            Tool::Text => return self.start_text(at, window, cx),
        });
        cx.notify();
    }

    fn on_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let q = from_point(ev.position);
        let k = self.view(window).map(|v| v.k);
        let Some(at) = self.to_px(q, window) else { return };
        let Some(ed) = &mut self.editor else { return };
        if let Some((from, pan)) = ed.panning {
            if ed.zoom.is_none() {
                ed.zoom = k;
            }
            ed.pan = pan + (q - from);
            return cx.notify();
        }
        if let Some(mv) = &mut ed.moving {
            if !mv.moved {
                if k.is_some_and(|k| (at - mv.from).len() * k < DRAG_SLOP) {
                    return;
                }
                mv.moved = true;
            }
            let shape = mv.was.moved(at - mv.from);
            if let Some(i) = ed.layer {
                ed.doc.set(i, shape);
            }
            return cx.notify();
        }
        if let Some(fd) = &mut ed.frame_drag {
            if !fd.moved {
                if k.is_some_and(|k| (at - fd.from).len() * k < DRAG_SLOP) {
                    return;
                }
                fd.moved = true;
            }
            let (grip, from, was) = (fd.grip, fd.from, fd.was);
            let (bw, bh) = (ed.size.0 as f32, ed.size.1 as f32);
            let rect = match grip {
                None if ed.pick == Some(Pick::Select) => model::norm(from, ed.clamp_to_image(at)),
                None => model::norm(from, at),
                Some(grip) => model::drag(was, grip, at - from, 1.),
            };
            let kept = match grip {
                // Corners keep the image's shape, unless ⇧ is held.
                Some(grip) if !ev.modifiers.shift => model::keep_aspect(was, grip, rect),
                _ => rect,
            };
            if let Some(ps) = &mut ed.pasted {
                ps.rect = kept;
            } else if ed.resize.is_some() {
                ed.resize = Some(kept);
            } else if ed.pick == Some(Pick::Select) {
                ed.area = Some(model::keep_inside(rect, bw, bh, grip == Some(Grip::Move)));
            } else {
                ed.area = Some(rect);
            }
            return cx.notify();
        }
        if ed.drag.is_none() {
            // As a press would take it: a layer before the picture's edge.
            let free = ed.tool.is_none() && ed.pick.is_none() && ed.pasted.is_none();
            let picked = ed.picked_image();
            let resize = k
                .filter(|_| free)
                .and_then(|k| picked.and_then(|(lo, hi)| model::grip_at(lo, hi, at, GRIP_REACH / k)))
                .filter(|g| *g != Grip::Move);
            // Over a shape, or the picked image: a drag would move it.
            let on_edge = k.is_some_and(|k| model::edge_grip(P::default(), ed.whole().1, at, EDGE_IN / k, EDGE_OUT / k).is_some());
            let over = resize.is_none()
                && free
                && k.and_then(|k| ed.layer_at(at, GRIP_REACH / k)).is_some_and(|i| {
                    let image = matches!(ed.doc.shapes[i], Shape::Image { .. });
                    !image || (Some(i) == ed.layer && !on_edge)
                });
            let edge = |k: f32| model::edge_grip(P::default(), ed.whole().1, at, EDGE_IN / k, EDGE_OUT / k);
            let hover = k.filter(|_| !over).and_then(|k| match ed.frame() {
                Some((lo, hi)) if ed.pick == Some(Pick::Select) => model::grip_at(lo, hi, at, GRIP_REACH / k).or_else(|| edge(k)),
                Some((lo, hi)) => model::grip_at(lo, hi, at, GRIP_REACH / k),
                None => edge(k),
            });
            if hover != ed.hover || over != ed.over_layer {
                ed.hover = hover;
                ed.over_layer = over;
                cx.notify();
            }
        }
        let Some(shape) = ed.drag.as_mut() else { return };
        match shape {
            Shape::Rect { b, .. } | Shape::Ellipse { b, .. } | Shape::Line { b, .. } | Shape::Arrow { b, .. } | Shape::Mosaic { b, .. } => {
                *b = at
            },
            Shape::Pen { points, .. } => {
                if points.last().is_none_or(|l| (*l - at).len() >= 1.) {
                    points.push(at)
                }
            },
            Shape::Text { .. } | Shape::Marker { .. } | Shape::Image { .. } | Shape::Erase { .. } => {},
        }
        cx.notify();
    }

    fn on_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        ed.panning = None;
        if let Some(mv) = ed.moving.take() {
            if mv.moved
                && let Some(i) = ed.layer
            {
                let shape = ed.doc.shapes[i].clone();
                ed.push(Op::Change { layer: i, shape });
            }
            return cx.notify();
        }
        if let Some(fd) = ed.frame_drag.take() {
            if let Some(rect) = ed.resize.take() {
                if fd.moved {
                    ed.finish_resize(rect);
                }
                return cx.notify();
            }
            if fd.quick {
                let big_enough = ed.area.is_some_and(|(lo, hi)| hi.x - lo.x >= 2. && hi.y - lo.y >= 2.);
                if fd.moved && big_enough {
                    ed.apply_crop();
                } else {
                    ed.area = None;
                    // A press on the picture's edge alone wasn't the crop tool.
                    if fd.grip.is_some() {
                        ed.pick = None;
                    }
                }
                if std::mem::take(&mut ed.select_after) {
                    ed.pick = Some(Pick::Select);
                    ed.area = None;
                }
                return cx.notify();
            }
            if ed.pasted.is_none() && ed.area.is_some_and(|(lo, hi)| hi.x - lo.x < 2. || hi.y - lo.y < 2.) {
                ed.area = None;
            }
            // A click away from the selection, not a drag: it is let go of;
            // the select tool stays, to pick another.
            if ed.pick == Some(Pick::Select) && fd.grip.is_none() && !fd.moved {
                ed.area = None;
            }
            return cx.notify();
        }
        let Some(shape) = ed.drag.take() else { return };
        let keep = match &shape {
            Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Line { a, b, .. }
            | Shape::Arrow { a, b, .. }
            | Shape::Mosaic { a, b } => !model::too_small(*a, *b, ed.doc.scale),
            _ => true,
        };
        if keep {
            ed.push(Op::Shape { shape });
        }
        cx.notify();
    }

    fn start_text(&mut self, at: P, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx));
        let sub = cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
            InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_text(window, cx),
            _ => {},
        });
        // After this click, which focuses the window when it ends.
        let handle = input.focus_handle(cx);
        window.defer(cx, move |window, cx| handle.focus(window, cx));
        if let Some(ed) = &mut self.editor {
            ed.text = Some(TextEdit { at, input, _sub: sub });
        }
        cx.notify();
    }

    fn commit_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ed) = &mut self.editor else { return };
        let Some(edit) = ed.text.take() else { return };
        let text = edit.input.read(cx).value().trim_end().to_string();
        if !text.is_empty() {
            let style = ed.style;
            ed.push(Op::Shape { shape: Shape::Text { at: edit.at, text, style } });
        }
        // Later, so the Enter that committed doesn't also reach the window.
        cx.defer_in(window, |this, window, cx| window.focus(&this.focus, cx));
        cx.notify();
    }

    fn pick_tool(&mut self, tool: Option<Tool>, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if let Some(ed) = &mut self.editor {
            ed.commit_paste();
            ed.tool = if ed.tool == tool { None } else { tool };
            ed.pick = None;
            ed.area = None;
            ed.unpick();
        }
        cx.notify();
    }

    /// Start picking an area to crop to or select, or stop.
    fn toggle_pick(&mut self, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if let Some(ed) = &mut self.editor {
            ed.commit_paste();
            ed.pick = if ed.pick == Some(pick) { None } else { Some(pick) };
            ed.area = None;
            ed.tool = None;
            ed.unpick();
        }
        cx.notify();
    }

    /// Select the whole capture.
    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_text(window, cx);
        if let Some(ed) = &mut self.editor {
            ed.commit_paste();
            ed.tool = None;
            ed.unpick();
            ed.pick = Some(Pick::Select);
            ed.area = Some(ed.whole());
        }
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let ks = &ev.keystroke;
        let m = ks.modifiers;
        let key = ks.key.as_str();
        if self.editor.is_some() && !self.expanded && self.viewing_trash() {
            match key {
                "=" | "+" if m.secondary() => self.zoom_by(ZOOM_STEP, None, window, cx),
                "-" if m.secondary() => self.zoom_by(1. / ZOOM_STEP, None, window, cx),
                "0" if m.secondary() => self.fit(cx),
                "1" if m.secondary() => self.actual_size(window, cx),
                "g" if !m.modified() => self.expand(true, window, cx),
                _ => {},
            }
            return;
        }
        let Some(ed) = self.editor.as_mut().filter(|_| !self.expanded) else {
            return self.grid_key(key, m, window, cx);
        };
        match key {
            "escape" if ed.drag.is_some() => ed.drag = None,
            "escape" if ed.moving.is_some() => {
                if let (Some(mv), Some(i)) = (ed.moving.take(), ed.layer) {
                    ed.doc.set(i, mv.was);
                }
            },
            "escape" if ed.pasted.is_some() => {
                ed.pasted = None;
                ed.frame_drag = None;
            },
            "escape" if ed.pick.is_some() => {
                ed.pick = None;
                ed.area = None;
                ed.frame_drag = None;
                ed.select_after = false;
            },
            "escape" if ed.tool.is_some() => ed.tool = None,
            "escape" if ed.resize.is_some() => {
                ed.resize = None;
                ed.frame_drag = None;
            },
            "escape" if ed.layer.is_some() => ed.unpick(),
            "escape" => return,
            "backspace" | "delete" if !m.modified() && ed.pick == Some(Pick::Select) && ed.area.is_some() => ed.delete_area(),
            "backspace" | "delete" if !m.modified() && ed.layer.is_some() && ed.moving.is_none() => ed.remove_layer(),
            "]" if m.secondary() && ed.moving.is_none() => ed.reorder_layer(1),
            "[" if m.secondary() && ed.moving.is_none() => ed.reorder_layer(-1),
            "l" if m.secondary() => return self.toggle_layers(cx),
            "g" if !m.modified() => return self.expand(true, window, cx),
            "enter" if ed.pasted.is_some() => ed.commit_paste(),
            "z" if m.secondary() && m.shift => {
                ed.redo();
            },
            "z" if m.secondary() => return self.undo(cx),
            "c" if m.secondary() => return self.copy(window, cx),
            "v" if m.secondary() && m.shift => return self.open_clipboard(cx),
            "o" if m.secondary() => return self.open_files(cx),
            "v" if m.secondary() => return self.paste(window, cx),
            "a" if m.secondary() => return self.select_all(window, cx),
            "s" | "e" if m.secondary() => return self.export(window, cx),
            "backspace" | "delete" if m.secondary() => {
                let id = ed.entry.id.clone();
                return self.delete(vec![id], window, cx);
            },
            "=" | "+" if m.secondary() => return self.zoom_by(ZOOM_STEP, None, window, cx),
            "-" if m.secondary() => return self.zoom_by(1. / ZOOM_STEP, None, window, cx),
            "0" if m.secondary() => return self.fit(cx),
            "1" if m.secondary() => return self.actual_size(window, cx),
            "left" if !m.modified() => return self.step(-1, window, cx),
            "right" if !m.modified() => return self.step(1, window, cx),
            "c" if !m.modified() => return self.toggle_pick(Pick::Crop, window, cx),
            "s" if !m.modified() => return self.toggle_pick(Pick::Select, window, cx),
            key if !m.modified() => {
                let tool = match key {
                    "r" => Tool::Rect,
                    "e" => Tool::Ellipse,
                    "l" => Tool::Line,
                    "a" => Tool::Arrow,
                    "p" => Tool::Pen,
                    "t" => Tool::Text,
                    "n" => Tool::Marker,
                    "m" => Tool::Mosaic,
                    _ => return,
                };
                return self.pick_tool(Some(tool), window, cx);
            },
            _ => return,
        }
        cx.notify();
    }

    fn grid_key(&mut self, key: &str, m: Modifiers, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            "escape" if !self.selected.is_empty() => self.selected.clear(),
            "escape" | "g" if self.editor.is_some() => self.expanded = false,
            "escape" => return window.remove_window(),
            "a" if m.secondary() => self.selected = self.entries.iter().map(|e| e.id.clone()).collect(),
            "c" if m.secondary() => return self.copy_picked(cx),
            "v" if m.secondary() => return self.open_clipboard(cx),
            "o" if m.secondary() => return self.open_files(cx),
            "s" | "e" if m.secondary() => return self.export_picked(cx),
            "backspace" | "delete" => return self.delete(self.selected.clone(), window, cx),
            "enter" => {
                if let [e] = self.picked().as_slice() {
                    self.expanded = false;
                    return self.open_capture(e.clone(), cx);
                }
                return;
            },
            _ => return,
        }
        cx.notify();
    }
}

impl Focusable for Library {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

enum Copied {
    Image(Vec<u8>),
    Files(Vec<std::path::PathBuf>),
}

/// Replace the clipboard with these files, one item each, as Finder does.
/// GPUI can't write file lists, so this goes to the pasteboard itself.
/// The image on the clipboard, or in the first image file copied.
fn clipboard_image(cx: &App) -> Option<RgbaImage> {
    cx.read_from_clipboard()?.into_entries().find_map(|e| match e {
        ClipboardEntry::Image(img) => raster::decode(img.bytes()),
        ClipboardEntry::ExternalPaths(paths) => paths.paths().iter().find_map(|f| store::open(f).ok()),
        _ => None,
    })
}

/// Delete the copied files once something else is put on the clipboard
/// (they can't be pasted from it any more).
fn forget_copied_later(cx: &mut App) {
    use std::sync::atomic::{AtomicU64, Ordering};
    /// Counts copies as files: a later copy takes over the watch.
    static COPIES: AtomicU64 = AtomicU64::new(0);
    let me = COPIES.fetch_add(1, Ordering::Relaxed) + 1;
    let count = pasteboard_count();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(COPIED_CHECK).await;
            if COPIES.load(Ordering::Relaxed) != me || !copied_dir().exists() {
                return;
            }
            if cx.update(|_| pasteboard_count()) != count {
                cx.background_executor().spawn(async { forget_copied() }).await;
                return;
            }
        }
    })
    .detach();
}

fn copy_files(paths: &[std::path::PathBuf]) {
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let items: Vec<_> = paths
        .iter()
        .filter_map(|path| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy())).absoluteString()?;
            let item = NSPasteboardItem::new();
            // SAFETY: a framework constant, always set.
            item.setString_forType(&url, unsafe { NSPasteboardTypeFileURL });
            Some(ProtocolObject::<dyn NSPasteboardWriting>::from_retained(item))
        })
        .collect();
    let board = NSPasteboard::generalPasteboard();
    board.clearContents();
    if !board.writeObjects(&NSArray::from_retained_slice(&items)) {
        log::warn!("the pasteboard took none of {} files", paths.len());
    }
}

/// The look of a pick box: a small ring, filled with a tick once picked.
fn pick_circle(d: Stateful<Div>, picked: bool) -> Stateful<Div> {
    let d = d.size(px(18.)).rounded_full().border_2().flex().items_center().justify_center().text_size(px(11.));
    if picked {
        d.bg(white()).border_color(white()).text_color(black()).child(icon(IconName::Check, 12.))
    } else {
        d.bg(rgba(0x00000066)).border_color(rgba(0xffffffcc)).hover(|s| s.bg(rgba(0xffffff40)))
    }
}

fn count(n: usize) -> String {
    if n == 1 { "1 capture".to_owned() } else { format!("{n} captures") }
}

impl Render for Library {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Every change shows, so this is where it is saved from.
        self.save_soon(cx);
        self.frame += 1;
        self.window_width.set(window.viewport_size().width.as_f32());
        // What the editor has open, in the title bar rather than the toolbar.
        let title = match self.editor.as_ref().filter(|_| !self.expanded) {
            Some(ed) => format!("{} · {} × {}", ed.entry.taken().format("%-d %b %Y, %H:%M:%S"), ed.size.0, ed.size.1),
            None => TITLE.to_owned(),
        };
        if title != self.title {
            window.set_window_title(&title);
            self.title = title;
        }
        let root = div()
            .id("library")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &crate::menus::CloseWindow, window, cx| {
                this.commit_text(window, cx);
                if let Some(ed) = this.editor.take() {
                    this.keep_changes(ed, cx);
                }
                toolbar::library_open(false, cx);
                dock::set(dock::Window::Library, false, cx);
                window.remove_window();
            }))
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .relative()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_size(px(13.));
        if self.editor.is_some() {
            let root = root.children(self.editor_view(window, cx));
            if self.expanded { root.child(self.grid_dialog(window, cx)) } else { root }
        } else if self.loaded && self.opening.is_none() && self.error.is_some() {
            // Why the library can't be shown.
            root.child(self.grid(cx))
        } else if self.loaded && self.opening.is_none() {
            // No capture to open: the editor's frame, empty.
            let root = root.children(self.empty_view(cx));
            if self.expanded { root.child(self.grid_dialog(window, cx)) } else { root }
        } else {
            // A capture is loading into the editor.
            root
        }
    }
}

impl Library {
    /// Every capture, filling the window: the library's view while no
    /// capture is open, or why there are none.
    fn grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(HEADER))
            .px(px(20.))
            .bg(rgb(PANEL))
            .border_b_1()
            .border_color(rgba(LINE))
            .child(self.grid_title(cx))
            .child(div().flex_1())
            .child(self.new_buttons(cx));
        let width = self.window_width.get();
        div().size_full().flex().flex_col().child(bar).child(self.grid_list(32., width, cx)).into_any_element()
    }

    /// The editor with no capture open: the side list (its folders and the
    /// trash), the buttons to take or open one, and a word in the middle.
    fn empty_view(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let bar = ui::bar().relative().h(px(BAR_H)).occlude().child(self.new_buttons(cx));
        let words = if self.trashed.is_empty() { "No captures yet" } else { "No captures, but some in the trash" };
        let (left, right) = (self.left(), self.right());
        let mut out = vec![
            canvas(|_, _, _| {}, |b, _, window, _| paint_dots(b, window)).absolute().top_0().left_0().size_full().into_any_element(),
            div().absolute().top(px(BAR_TOP)).left(px(left)).right(px(right)).flex().justify_center().child(bar).into_any_element(),
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(left))
                .right(px(right))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(words))
                .child(div().text_size(px(12.)).text_color(rgb(DIM)).child("Take one with the buttons above, or paste an image (⇧⌘V)."))
                .into_any_element(),
            self.side(cx),
        ];
        // The layers' panel stays where it is, empty, so nothing moves.
        if self.layers_shown {
            out.push(
                div()
                    .absolute()
                    .top(px(FLOAT))
                    .right(px(FLOAT))
                    .bottom(px(FLOAT))
                    .w(px(LAYERS))
                    .rounded(px(10.))
                    .overflow_hidden()
                    .bg(rgba(ui::BAR_BG))
                    .border_1()
                    .border_color(rgba(ui::BAR_LINE))
                    .occlude()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(LAYERS_TOP))
                            .pl(px(12.))
                            .pr(px(6.))
                            .border_b_1()
                            .border_color(rgba(LINE))
                            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Layers"))
                            .child(
                                icon_button("layers-close", IconName::X, false, true)
                                    .size(px(24.))
                                    .tooltip(tip("Hide the layers (⌘L)"))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_layers(cx))),
                            ),
                    )
                    .into_any_element(),
            );
        }
        out
    }

    /// Every capture, in a dialog over the editor; a click on one opens it.
    fn grid_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let vp = window.viewport_size();
        let w = (vp.width.as_f32() - 120.).clamp(320., 1180.);
        let h = (vp.height.as_f32() - 96.).max(240.);
        let head = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(52.))
            .pl(px(20.))
            .pr(px(10.))
            .border_b_1()
            .border_color(rgba(LINE))
            .child(self.grid_title(cx))
            .child(div().flex_1())
            .child(
                icon_button("grid-close", IconName::X, false, true)
                    .tooltip(tip("Close (Esc)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.expand(false, window, cx))),
            );
        let panel = div()
            .id("grid-dialog")
            .w(px(w))
            .h(px(h))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgb(PANEL))
            .border_1()
            .border_color(rgba(ui::BAR_LINE))
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(head)
            .child(self.grid_list(20., w, cx));
        // A click beside it closes it.
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, window, cx| this.expand(false, window, cx)))
            .child(panel)
            .into_any_element()
    }

    /// How many captures there are, or what to do with those picked.
    fn grid_title(&self, cx: &mut Context<Self>) -> AnyElement {
        if !self.selected.is_empty() {
            return self.list_status(cx);
        }
        div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(count(self.entries.len())).into_any_element()
    }

    /// The captures by folder, `pad` in from the sides of an area `width`
    /// points wide; or why there are none. Like the side list, only the
    /// rows in sight are made.
    fn grid_list(&self, pad: f32, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let message = |text: String| div().flex_1().min_h_0().px(px(pad)).pt(px(4.)).child(empty(&text)).into_any_element();
        if let Some(e) = &self.error {
            return message(format!("The library can't be opened: {e}"));
        }
        if self.entries.is_empty() {
            return message("No captures yet. Take one from the toolbar or the menu bar.".to_owned());
        }
        // As many cards to a row as fit.
        let cols = (((width - 2. * pad + CARD_GAP) / (CARD_W + 16. + CARD_GAP)).floor() as usize).max(1);
        let key = self.rows_key(cols);
        if self.grid_key.borrow().as_ref() != Some(&key) {
            let index: HashMap<&str, usize> = self.entries.iter().enumerate().map(|(i, e)| (e.id.as_str(), i)).collect();
            let mut rows = Vec::new();
            for (folder, name, entries) in self.groups() {
                let cards: Vec<(usize, Entry)> =
                    entries.iter().map(|e| (index.get(e.id.as_str()).copied().unwrap_or(0), e.clone())).collect();
                rows.push(GridRow::Folder { id: folder, name, entries: Rc::new(entries) });
                rows.extend(cards.chunks(cols).map(|c| GridRow::Cards(c.to_vec())));
            }
            rows.push(GridRow::End);
            sync_list(&self.grid_list, &self.grid_rows, rows);
            *self.grid_key.borrow_mut() = Some(key);
        }
        list(
            self.grid_list.clone(),
            cx.processor(move |this: &mut Self, ix: usize, _window, cx| {
                let el = this.grid_row(ix, pad, cx);
                this.fetch_wanted(cx);
                el
            }),
        )
        .flex_1()
        .min_h_0()
        .into_any_element()
    }

    /// Row `ix` of the grid, `pad` in from the sides.
    fn grid_row(&self, ix: usize, pad: f32, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.grid_rows.borrow().get(ix).cloned() else { return div().into_any_element() };
        let row = match row {
            GridRow::Folder { id, name, entries } => {
                div().pt(px(12.)).pb(px(6.)).child(self.folder_header("grid", id, &name, &entries, false, cx))
            },
            GridRow::Cards(cards) => {
                div().flex().gap(px(CARD_GAP)).pb(px(CARD_GAP)).children(cards.into_iter().map(|(i, e)| self.card(i, e, cx)))
            },
            GridRow::End => div().h(px(pad)),
        };
        row.px(px(pad)).into_any_element()
    }

    fn card(&self, i: usize, e: Entry, cx: &mut Context<Self>) -> AnyElement {
        let opening = self.opening.as_deref() == Some(&e.id);
        let picked = self.selected.contains(&e.id);
        let pic = self.thumb_element(&e.id, CARD_W, CARD_PIC_H, 24.);
        let time = e.taken().format("%H:%M:%S").to_string();
        let dragged = self.dragged(&e.id, time.clone());
        div()
            .id(SharedString::from(e.id.clone()))
            .on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()))
            .group("card")
            .relative()
            .w(px(CARD_W + 16.))
            .p(px(8.))
            .rounded(px(10.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff12)))
            .when(opening || picked, |d| d.bg(rgba(0xffffff1f)))
            .child(
                div()
                    .w(px(CARD_W))
                    .h(px(CARD_PIC_H))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .bg(rgb(PANEL))
                    .border_2()
                    .border_color(if picked { white() } else { transparent_black() })
                    .relative()
                    .overflow_hidden()
                    .child(pic)
                    .child(self.pick_box(i, &e.id, 6., cx))
                    .child(self.card_actions(&e, 6., cx)),
            )
            .child(div().text_size(px(12.)).text_color(rgb(DIM)).child(time))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| this.click_card(i, ev.modifiers(), cx)))
            .into_any_element()
    }

    fn editor_view(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(ed) = &self.editor else { return Vec::new() };
        let Some(view) = self.view(window) else { return Vec::new() };
        let (iw, ih) = (ed.size.0 as f32 * view.k, ed.size.1 as f32 * view.k);
        // The layers shown, bottom first, an image with its pixels; the
        // picked image where it is being dragged to.
        let mut layers: Vec<(Shape, Option<Arc<RenderImage>>)> = Vec::new();
        for (i, s) in ed.doc.shapes.iter().enumerate().filter(|(i, _)| !ed.doc.is_hidden(*i)) {
            match s {
                Shape::Image { image, a, b } => {
                    let (a, b) = ed.resize.filter(|_| ed.layer == Some(i)).unwrap_or((*a, *b));
                    layers.push((Shape::Image { image: *image, a, b }, ed.pictures.get(image).cloned()));
                },
                _ => layers.push((s.clone(), None)),
            }
        }
        layers.extend(ed.drag.clone().map(|s| (s, None)));
        // The picked shape's box, a little out from it; an image has its
        // handles instead.
        let outline = ed.layer.and_then(|i| ed.doc.shapes.get(i)).filter(|s| !matches!(s, Shape::Image { .. })).map(|s| {
            let (lo, hi) = raster::bounds(s, ed.doc.scale, raster::font());
            let pad = p(3., 3.);
            (to_point(from_point(view.pt(lo)) - pad), to_point(from_point(view.pt(hi)) + pad))
        });
        let under = ed.under.clone().unwrap_or_else(|| ed.original.clone());
        // An image pasted and not laid down yet, over everything; it may
        // reach past the canvas.
        let pasted = ed.pasted.as_ref().map(|ps| (Bounds::from_corners(view.pt(ps.rect.0), view.pt(ps.rect.1)), ps.picture.clone()));
        let picture_bounds = Bounds::new(to_point(view.origin), size(px(iw), px(ih)));
        let screen = |(lo, hi): (P, P)| (view.pt(lo), view.pt(hi));
        let frame = match (&ed.pasted, ed.pick) {
            (Some(ps), _) => Some(FrameLook::Pasted(screen(ps.rect))),
            (None, Some(Pick::Crop)) => Some(FrameLook::Crop(ed.area.map(screen))),
            (None, Some(Pick::Select)) => ed.area.map(|a| FrameLook::Select(screen(a))),
            (None, None) => ed.frame().map(|r| FrameLook::Pasted(screen(r))),
        };
        let vp = window.viewport_size();
        let stage_mask = Bounds::from_corners(point(px(0.), px(0.)), point(vp.width, vp.height));
        let canvas_el = canvas(
            |_, _, _| {},
            move |_, _, window, cx| {
                window.with_content_mask(Some(ContentMask { bounds: stage_mask }), |window| {
                    window.with_content_mask(Some(ContentMask { bounds: picture_bounds }), |window| {
                        // The canvas is blank paper under its layers.
                        window.paint_quad(fill(picture_bounds, hsla(raster::PAPER)));
                        for (s, pic) in &layers {
                            match (s, pic) {
                                (Shape::Image { a, b, .. }, Some(pic)) => {
                                    let (lo, hi) = model::norm(*a, *b);
                                    let at = Bounds::from_corners(view.pt(lo), view.pt(hi));
                                    let _ = window.paint_image(at, at, Corners::default(), pic.clone(), 0, false);
                                },
                                (Shape::Image { .. }, None) => {},
                                _ => paint::shape(view, s, &under, window, cx),
                            }
                        }
                    });
                    if let Some((at, pic)) = &pasted {
                        let _ = window.paint_image(*at, *at, Corners::default(), pic.clone(), 0, false);
                    }
                    if let Some((lo, hi)) = outline {
                        paint_outline(lo, hi, window);
                    }
                    if let Some(frame) = frame {
                        paint_frame(picture_bounds, frame, window);
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let grip = ed.frame_drag.as_ref().map_or(ed.hover, |fd| fd.grip);
        let cursor = if let Some(grip) = grip {
            grip_cursor(grip, ed.frame_drag.is_some())
        } else if ed.moving.as_ref().is_some_and(|mv| mv.moved) {
            CursorStyle::ClosedHand
        } else if ed.pasted.is_some() || ed.over_layer || ed.moving.is_some() {
            CursorStyle::Arrow
        } else if ed.pick.is_some() || ed.tool.is_some() {
            CursorStyle::Crosshair
        } else if ed.panning.is_some() {
            CursorStyle::ClosedHand
        } else {
            CursorStyle::OpenHand
        };
        let stage = div()
            .id("stage")
            .absolute()
            .inset_0()
            .overflow_hidden()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_down))
            .on_mouse_move(cx.listener(Self::on_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_pinch(cx.listener(Self::on_pinch))
            .child(canvas(|_, _, _| {}, |b, _, window, _| paint_dots(b, window)).absolute().top_0().left_0().size_full());
        let in_trash = self.viewing_trash();
        let bar = if in_trash { self.trash_bar(ed, cx) } else { self.header(ed, cx) };
        let mut out = vec![stage.into_any_element(), canvas_el.into_any_element(), self.side(cx), bar];
        if in_trash {
            // Nothing to mark up, so no layers.
        } else if self.layers_shown {
            out.push(self.layers(ed, cx));
        } else {
            // Put away, the layers come back from the stage's top right.
            out.push(
                div()
                    .absolute()
                    .top(px(BAR_TOP))
                    .right(px(FLOAT))
                    .child(
                        ui::bar().relative().h(px(34.)).occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
                            icon_button("layers", IconName::Layers, false, true)
                                .tooltip(tip("Layers (⌘L)"))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_layers(cx))),
                        ),
                    )
                    .into_any_element(),
            );
        }
        out.extend(self.style_bar(ed, cx));
        out.push(self.corner(ed, view, cx));
        if let Some(edit) = &ed.text {
            let at = view.origin + edit.at.scale(view.k);
            let font = ed.style.font() * view.zoom();
            let input = Input::new(&edit.input).appearance(false).text_color(hsla(ed.style.color)).text_size(px(font));
            out.push(
                div()
                    .absolute()
                    .left(px(at.x))
                    .top(px(at.y - 4.))
                    .w(px((view.origin.x + iw - at.x).max(120.)))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(input)
                    .into_any_element(),
            );
        }
        out
    }

    /// The stage's bottom right corner: the size of what is being framed
    /// (the pasted image, the area picked, or the picked image), and the
    /// zoom.
    fn corner(&self, ed: &Editor, view: View, cx: &mut Context<Self>) -> AnyElement {
        let framed = match (&ed.pasted, ed.pick) {
            (Some(ps), _) => Some(ps.rect),
            (None, Some(_)) => ed.area,
            (None, None) if ed.tool.is_none() => ed.resize.or_else(|| ed.picked_image()),
            (None, None) => None,
        };
        let size = framed.map(|(lo, hi)| {
            let (w, h) = ((hi.x - lo.x).round().max(1.), (hi.y - lo.y).round().max(1.));
            div().px(px(8.)).py(px(3.)).rounded(px(6.)).bg(rgba(0x000000c0)).text_size(px(12.)).child(format!("{w} × {h}"))
        });
        let zoom_label: SharedString = format!("{:.0}%", view.k * ed.doc.scale * 100.).into();
        let zoom = ui::bar()
            .relative()
            .h(px(34.))
            .child(
                icon_button("zoom-out", IconName::ZoomOut, false, true)
                    .tooltip(tip("Zoom out (⌘−)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.zoom_by(1. / ZOOM_STEP, None, window, cx))),
            )
            .child(
                button("zoom", zoom_label, false, true).w(px(52.)).text_size(px(12.)).tooltip(tip("Fit (⌘0) · Actual size (⌘1)")).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        if this.editor.as_ref().is_some_and(|ed| ed.zoom.is_some()) { this.fit(cx) } else { this.actual_size(window, cx) }
                    }),
                ),
            )
            .child(
                icon_button("zoom-in", IconName::ZoomIn, false, true)
                    .tooltip(tip("Zoom in (⌘+)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.zoom_by(ZOOM_STEP, None, window, cx))),
            );
        div()
            .absolute()
            .bottom(px(12.))
            .right(px(self.right() + 12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_color(rgb(TEXT))
            .children(size)
            .child(zoom.occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()))
            .into_any_element()
    }

    /// The captures beside the editor, by day; a click opens one.
    /// Copy and delete buttons in the top left corner of a capture's
    /// picture, shown while the pointer is over the capture.
    fn card_actions(&self, e: &Entry, inset: f32, cx: &mut Context<Self>) -> AnyElement {
        let action = |id: String, name: IconName| {
            div()
                .id(SharedString::from(id))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(rgba(0x000000a0))
                .text_color(white())
                .cursor_pointer()
                .hover(|s| s.bg(rgba(0x000000e0)))
                .child(icon(name, 13.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let (copied, deleted) = (e.clone(), e.id.clone());
        div()
            .absolute()
            .top(px(inset))
            .left(px(inset))
            .flex()
            .gap(px(4.))
            .opacity(0.)
            .group_hover("card", |s| s.opacity(1.))
            .child(action(format!("copy-{}", e.id), IconName::Copy).tooltip(tip("Copy")).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.copy_entries(vec![copied.clone()], cx);
                },
            )))
            .child(action(format!("delete-{}", e.id), IconName::Trash).tooltip(tip("Delete")).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    cx.stop_propagation();
                    this.delete(vec![deleted.clone()], window, cx);
                },
            )))
            .into_any_element()
    }

    /// The round pick box in the corner of a capture's picture: a click on
    /// it picks the capture (⇧ for a range) instead of opening it.
    fn pick_box(&self, i: usize, id: &str, inset: f32, cx: &mut Context<Self>) -> AnyElement {
        self.pick_dot(i, id, cx).absolute().top(px(inset)).right(px(inset)).into_any_element()
    }

    /// Capture `i`'s pick box, placed by the caller.
    fn pick_dot(&self, i: usize, id: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let picked = self.selected.iter().any(|s| s == id);
        // Out of the way until wanted: on the capture under the pointer,
        // or on all of them once picking has started.
        let shown = !self.selected.is_empty();
        div()
            .id(SharedString::from(format!("pick-{id}")))
            .flex_none()
            .map(|d| pick_circle(d, picked))
            .when(!shown, |d| d.opacity(0.).group_hover("card", |s| s.opacity(1.)))
            .tooltip(tip(if picked { "Unpick" } else { "Pick" }))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.toggle_card(i, ev.modifiers().shift, cx);
            }))
    }

    /// Beside a day's title: pick all of that day's captures, or none when
    /// they all are.
    fn day_pick(&self, at: &str, title: &str, entries: &[Entry], cx: &mut Context<Self>) -> AnyElement {
        let ids: Vec<String> = entries.iter().map(|e| e.id.clone()).collect();
        let all = ids.iter().all(|id| self.selected.contains(id));
        let shown = !self.selected.is_empty();
        div()
            .id(SharedString::from(format!("{at}-day-{title}")))
            .cursor_pointer()
            .map(|d| pick_circle(d, all))
            .when(!shown, |d| d.opacity(0.).group_hover("day", |s| s.opacity(1.)))
            .tooltip(tip(if all { "Unpick them all" } else { "Pick them all" }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                if all {
                    this.selected.retain(|id| !ids.contains(id));
                } else {
                    for id in &ids {
                        if !this.selected.contains(id) {
                            this.selected.push(id.clone());
                        }
                    }
                }
                cx.notify();
            }))
            .into_any_element()
    }

    /// Above the side list while captures are picked: what to do with them.
    /// Above the captures while some are picked, in the grid and beside
    /// the editor alike: how many, and what to do with them.
    fn picked_bar(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .h(px(40.))
            .px(px(6.))
            .border_b_1()
            .border_color(rgba(LINE))
            .child(icon_button("side-clear", IconName::X, false, true).tooltip(tip("Pick none")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.selected.clear();
                    cx.notify();
                },
            )))
            .child(
                div()
                    .h(px(18.))
                    .min_w(px(18.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(white())
                    .text_color(black())
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.selected.len().to_string()),
            )
            .child(sep())
            .child(
                icon_button("side-copy", IconName::Copy, false, true)
                    .tooltip(tip("Copy the picked captures"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.copy_picked(cx))),
            )
            .child(
                icon_button("side-export", IconName::Download, false, true)
                    .tooltip(tip("Export the picked captures as PNG files…"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.export_picked(cx))),
            )
            .child(icon_button("side-delete", IconName::Trash, false, true).tooltip(tip("Delete the picked captures")).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| {
                    let ids = this.selected.clone();
                    this.delete(ids, window, cx)
                }),
            ))
    }

    /// New captures: taken now (the window steps aside), opened from
    /// image files, or from the image on the clipboard.
    fn new_buttons(&self, cx: &mut Context<Self>) -> Div {
        let shortcuts = &prefs::get(cx).shortcuts;
        let mut row = div().relative().flex().flex_none().items_center().gap(px(2.));
        for (action, mode, name) in [
            (Action::Region, capture::Mode::Region, IconName::Scan),
            (Action::Window, capture::Mode::Window, IconName::AppWindow),
            (Action::Screen, capture::Mode::Screen, IconName::Monitor),
            (Action::Scroll, capture::Mode::Scroll, IconName::ScrollText),
        ] {
            let hint: SharedString = match shortcuts.get(action) {
                Some(s) => format!("{} ({})", action.title(), s.label()).into(),
                None => action.title().into(),
            };
            row = row.child(
                icon_button(SharedString::from(format!("capture-{}", action.title())), name, false, true)
                    .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                    // After this click: the window is put away, which can't
                    // be done from inside its own event.
                    .on_click(move |_, _, cx| cx.defer(move |cx| capture::start_from_library(mode, cx))),
            );
        }
        row.child(sep())
            .child(
                icon_button("open-files", IconName::FolderOpen, false, true)
                    .tooltip(tip("Open image files… (⌘O)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_files(cx))),
            )
            .child(
                icon_button("from-clipboard", IconName::ClipboardPaste, false, true)
                    .tooltip(tip("New from the clipboard (⇧⌘V)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.open_clipboard(cx))),
            )
    }

    fn open_clipboard(&mut self, cx: &mut Context<Self>) {
        open_clipboard(cx);
    }

    fn open_files(&mut self, cx: &mut Context<Self>) {
        open_files(cx);
    }

    /// The side list's top, beside the toolbar: what to do with the
    /// captures picked, and the button that grows the list to fill the
    /// window. (← and → step through the captures.)
    fn side_top(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_none()
            .items_center()
            // As the layers' top, across from it.
            .h(px(LAYERS_TOP))
            .pl(px(12.))
            .pr(px(6.))
            .border_b_1()
            .border_color(rgba(LINE))
            .map(|d| {
                if self.selected.is_empty() {
                    d.child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(count(self.entries.len())))
                } else {
                    d.child(self.list_status(cx))
                }
            })
            .child(div().flex_1())
            .child(
                icon_button("new-folder", IconName::FolderPlus, false, true)
                    .size(px(24.))
                    .tooltip(tip("New folder"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.new_folder(window, cx))),
            )
            .child(
                icon_button("expand", IconName::Maximize2, false, true)
                    .size(px(24.))
                    .tooltip(tip("All captures (G)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.expand(true, window, cx))),
            )
    }

    /// The trash's node, last in the side list: captures dropped on it go
    /// in; a click opens it, to show what is in it.
    fn trash_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let n = self.trashed.len();
        let open = self.trash_open && n > 0;
        div()
            .id("trash")
            .group("day")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(30.))
            .mt(px(6.))
            .pl(px(4.))
            .pr(px(6.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(|s| s.bg(rgba(0xffffff0d)))
            .drag_over::<Dragged>(|s, _, _, _| s.bg(rgba(0xffffff26)))
            .on_drop(cx.listener(|this, d: &Dragged, window, cx| this.delete(d.ids.clone(), window, cx)))
            .child(div().text_color(rgb(DIM)).child(icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, 14.)))
            .child(div().text_color(rgb(DIM)).child(icon(IconName::Trash, 14.)))
            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Trash"))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .min_w(px(18.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(DIM))
                            .when(n > 0, |d| d.group_hover("day", |s| s.opacity(0.)))
                            .child(n.to_string()),
                    )
                    .when(n > 0, |d| {
                        let all: Vec<String> = self.trashed.iter().map(|e| e.id.clone()).collect();
                        d.child(
                            div()
                                .absolute()
                                .right_0()
                                .top_0()
                                .bottom_0()
                                .flex()
                                .items_center()
                                .opacity(0.)
                                .group_hover("day", |s| s.opacity(1.))
                                .child(trash_action("trash-empty".into(), IconName::Trash, "Empty the trash").on_click(cx.listener(
                                    move |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.empty_trash(all.clone(), window, cx);
                                    },
                                ))),
                        )
                    }),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.trash_open = !this.trash_open;
                cx.notify();
            }))
            .into_any_element()
    }

    /// A capture in the open trash: to open, put back or delete for good.
    fn trashed_row(&self, e: &Entry, open_id: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let current = open_id == Some(e.id.as_str());
        let pic = self.thumb_element(&e.id, ROW_PIC_W, ROW_PIC_H, 16.);
        let (back, gone, open_it) = (vec![e.id.clone()], vec![e.id.clone()], e.clone());
        div()
            .id(SharedString::from(format!("trashed-{}", e.id)))
            .group("card")
            .cursor_pointer()
            .ml(px(11.))
            .pl(px(8.))
            .border_l_1()
            .border_color(rgba(LINE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .p(px(3.))
                    .rounded(px(6.))
                    .map(|d| if current { d.bg(rgba(0xffffff26)) } else { d.group_hover("card", |s| s.bg(rgba(0xffffff12))) })
                    .child(
                        div()
                            .w(px(ROW_PIC_W))
                            .h(px(ROW_PIC_H))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .bg(rgb(BG))
                            .overflow_hidden()
                            .opacity(0.6)
                            .child(pic),
                    )
                    .child(div().flex_1().text_size(px(12.)).text_color(rgb(DIM)).child(e.taken().format("%H:%M:%S").to_string()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .pr(px(4.))
                            .opacity(0.)
                            .group_hover("card", |s| s.opacity(1.))
                            .child(trash_action(format!("restore-{}", e.id), IconName::Undo2, "Put back").on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.restore(back.clone(), cx);
                                },
                            )))
                            .child(trash_action(format!("forget-{}", e.id), IconName::X, "Delete for good").on_click(cx.listener(
                                move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.delete_for_good(gone.clone(), window, cx);
                                },
                            ))),
                    ),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.switch_to(open_it.clone(), window, cx)))
            .into_any_element()
    }

    /// Left in the side list's top: what to do with the captures picked,
    /// or, with none picked, how many captures there are.
    fn list_status(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.selected.is_empty() {
            div()
                .px(px(6.))
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(TEXT))
                .child(count(self.entries.len()))
                .into_any_element()
        } else {
            self.picked_bar(cx).h_full().px_0().border_0().into_any_element()
        }
    }

    /// What the side list's and grid's rows are made from, `cols` cards
    /// across (the grid's): while it is the same, so are they.
    fn rows_key(&self, cols: usize) -> RowsKey {
        let mut folded: Vec<u64> = self.folded.iter().copied().collect();
        folded.sort_unstable();
        RowsKey { listed: self.listed, folders: self.folders.rev(), folded, trash_open: self.trash_open, cols }
    }

    /// The rows of the side list as it is now: the captures by folder,
    /// then the trash.
    fn side_rows(&self) -> Vec<SideRow> {
        let index: HashMap<&str, usize> = self.entries.iter().enumerate().map(|(i, e)| (e.id.as_str(), i)).collect();
        let mut rows = Vec::new();
        for (folder, name, entries) in self.groups() {
            let folded = self.folded.contains(&folder);
            let entries = Rc::new(entries);
            rows.push(SideRow::Folder { id: folder, name, entries: entries.clone() });
            if !folded {
                rows.extend(
                    entries.iter().map(|e| SideRow::Capture { at: index.get(e.id.as_str()).copied().unwrap_or(0), entry: e.clone() }),
                );
            }
        }
        rows.push(SideRow::TrashHeader);
        if self.trash_open {
            rows.extend(self.trashed.iter().cloned().map(SideRow::Trashed));
        }
        rows.push(SideRow::End);
        rows
    }

    /// The captures by folder, and the trash. Only the rows in sight are
    /// made, so a library of thousands draws as fast as a short one.
    fn side(&self, cx: &mut Context<Self>) -> AnyElement {
        // Laid out again only when what it lists changed, not on every
        // frame (a drag on the canvas draws many).
        let key = self.rows_key(0);
        if self.side_key.borrow().as_ref() != Some(&key) {
            sync_list(&self.side_list, &self.side_rows, self.side_rows());
            *self.side_key.borrow_mut() = Some(key);
        }
        let open_id = self.editor.as_ref().map(|ed| ed.entry.id.as_str());
        let reveal = self
            .reveal
            .take()
            .then(|| {
                self.side_rows
                    .borrow()
                    .iter()
                    .position(|r| matches!(r, SideRow::Capture { entry, .. } if Some(entry.id.as_str()) == open_id))
            })
            .flatten();
        if let Some(ix) = reveal {
            self.side_list.scroll_to_reveal_item(ix);
        }
        let list = list(
            self.side_list.clone(),
            cx.processor(|this: &mut Self, ix: usize, _window, cx| {
                let el = this.side_row(ix, cx);
                this.fetch_wanted(cx);
                el
            }),
        )
        .flex_1()
        .min_h_0();
        div()
            .absolute()
            .top(px(FLOAT))
            .left(px(FLOAT))
            .bottom(px(FLOAT))
            .w(px(SIDE))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .overflow_hidden()
            .bg(rgba(ui::BAR_BG))
            .border_1()
            .border_color(rgba(ui::BAR_LINE))
            .occlude()
            .child(self.side_top(cx))
            .child(list)
            .into_any_element()
    }

    /// Row `ix` of the side list.
    fn side_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.side_rows.borrow().get(ix).cloned() else { return div().into_any_element() };
        let open_id = self.editor.as_ref().map(|ed| ed.entry.id.clone());
        let open_id = open_id.as_deref();
        let el = match row {
            SideRow::Folder { id, name, entries } => self.folder_header("side", id, &name, &entries, true, cx),
            SideRow::Capture { at, entry } => self.side_capture(at, entry, open_id, cx),
            SideRow::TrashHeader => self.trash_header(cx),
            SideRow::Trashed(e) => self.trashed_row(&e, open_id, cx),
            SideRow::End => return div().h(px(16.)).into_any_element(),
        };
        div().px(px(8.)).child(el).into_any_element()
    }

    /// A capture's row in the side list, under its folder.
    fn side_capture(&self, at: usize, e: Entry, open_id: Option<&str>, cx: &mut Context<Self>) -> AnyElement {
        let current = open_id == Some(e.id.as_str()) || self.opening.as_deref() == Some(&e.id);
        let (pw, ph) = (ROW_PIC_W, ROW_PIC_H);
        let pic = self.thumb_element(&e.id, pw, ph, 16.);
        let time = e.taken().format("%H:%M:%S").to_string();
        let frame = div()
            .w(px(pw))
            .h(px(ph))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .bg(rgb(BG))
            .relative()
            .overflow_hidden()
            .child(pic);
        let dragged = self.dragged(&e.id, time.clone());
        let item = div()
            .id(SharedString::from(format!("side-{}", e.id)))
            .group("card")
            .relative()
            .cursor_pointer()
            .on_drag(dragged, |d, _, _, cx| cx.new(|_| d.clone()));
        // A row under its folder, a line down the left joining them:
        // the picture small, the time, its pick box.
        let item = item.ml(px(11.)).pl(px(8.)).border_l_1().border_color(rgba(LINE)).child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .p(px(3.))
                .rounded(px(6.))
                .map(|d| if current { d.bg(rgba(0xffffff26)) } else { d.group_hover("card", |s| s.bg(rgba(0xffffff12))) })
                .child(frame)
                .child(div().flex_1().text_size(px(12.)).text_color(if current { rgb(TEXT) } else { rgb(DIM) }).child(time))
                // Copy, trash and pick box spaced as on the folder header,
                // so the trash buttons line up down the tree.
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .pr(px(3.))
                        .child(
                            div()
                                .id(SharedString::from(format!("row-copy-{}", e.id)))
                                .flex_none()
                                .opacity(0.)
                                .group_hover("card", |s| s.opacity(1.))
                                .text_color(rgb(DIM))
                                .hover(|s| s.text_color(rgb(TEXT)))
                                .tooltip(tip("Copy"))
                                .on_click({
                                    let entry = e.clone();
                                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.copy_entries(vec![entry.clone()], cx);
                                    })
                                })
                                .child(icon(IconName::Copy, 13.)),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("row-delete-{}", e.id)))
                                .flex_none()
                                .opacity(0.)
                                .group_hover("card", |s| s.opacity(1.))
                                .text_color(rgb(DIM))
                                .hover(|s| s.text_color(rgb(TEXT)))
                                .tooltip(tip("Delete"))
                                .on_click({
                                    let id = e.id.clone();
                                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.delete(vec![id.clone()], window, cx);
                                    })
                                })
                                .child(icon(IconName::Trash, 13.)),
                        )
                        .child(self.pick_dot(at, &e.id, cx)),
                ),
        );
        item.on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
            let m = ev.modifiers();
            if m.shift || m.secondary() {
                this.toggle_card(at, m.shift, cx);
            } else {
                this.switch_to(e.clone(), window, cx);
            }
        }))
        .into_any_element()
    }

    /// The editor's toolbar, right of the side list: marking up in the
    /// middle, what to do with the capture on the right.
    /// Whether the capture open is in the trash.
    fn viewing_trash(&self) -> bool {
        self.editor.as_ref().is_some_and(|ed| self.trashed.iter().any(|e| e.id == ed.entry.id))
    }

    /// In place of the toolbar for a capture in the trash: only to put it
    /// back.
    fn trash_bar(&self, ed: &Editor, cx: &mut Context<Self>) -> AnyElement {
        let id = ed.entry.id.clone();
        let bar = ui::bar()
            .relative()
            .h(px(BAR_H))
            .gap(px(8.))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().pl(px(6.)).text_size(px(12.)).text_color(rgb(DIM)).child("In the trash"))
            .child(
                pill("put-back", "Put back", true)
                    .tooltip(tip("Back to the folder it was in"))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.restore(vec![id.clone()], cx))),
            );
        div()
            .absolute()
            .top(px(BAR_TOP))
            .left(px(self.left()))
            .right(px(self.right()))
            .flex()
            .justify_center()
            .child(bar)
            .into_any_element()
    }

    fn header(&self, ed: &Editor, cx: &mut Context<Self>) -> AnyElement {
        // Picking an area first, then the drawing tools, then undo.
        let mut tools = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(
                icon_button("select", IconName::SquareDashed, ed.pick == Some(Pick::Select), true)
                    .tooltip(tip("Select, to copy (S)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_pick(Pick::Select, window, cx))),
            )
            .child(
                icon_button("crop", IconName::Crop, ed.pick == Some(Pick::Crop), true)
                    .tooltip(tip("Crop (C) · or drag the picture's edge, out to add room"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_pick(Pick::Crop, window, cx))),
            )
            .child(sep());
        for (i, tool) in Tool::ALL.into_iter().enumerate() {
            let (name, hint) = tool_label(tool);
            tools = tools.child(
                icon_button(("tool", i), name, ed.tool == Some(tool), true)
                    .tooltip(tip(hint))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.pick_tool(Some(tool), window, cx))),
            );
        }
        tools = tools
            .child(sep())
            .child(
                icon_button("undo", IconName::Undo2, false, ed.can_undo())
                    .tooltip(tip("Undo (⌘Z)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.undo(cx))),
            )
            .child(icon_button("redo", IconName::Redo2, false, ed.can_redo()).tooltip(tip("Redo (⌘⇧Z)")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    if let Some(ed) = &mut this.editor {
                        ed.redo();
                    }
                    cx.notify();
                },
            )));

        let output = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(icon_button("delete", IconName::Trash, false, true).tooltip(tip("Delete the capture (⌘⌫)")).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| {
                    if let Some(id) = this.editor.as_ref().map(|ed| ed.entry.id.clone()) {
                        this.delete(vec![id], window, cx)
                    }
                },
            )))
            .child(
                icon_button("export", IconName::Download, false, true)
                    .tooltip(tip("Export as a PNG file… (⌘S)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.export(window, cx))),
            )
            .child(div().w(px(6.)))
            .child(
                pill("copy", "Copy", true)
                    .tooltip(tip("Copy the capture, or the selection (⌘C)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.copy(window, cx))),
            );

        let bar = ui::bar()
            .relative()
            .h(px(BAR_H))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.new_buttons(cx))
            .child(sep())
            .child(tools)
            .child(sep())
            .child(output);
        // Floating over the stage's top, in the middle of it.
        div()
            .absolute()
            .top(px(BAR_TOP))
            .left(px(self.left()))
            .right(px(self.right()))
            .flex()
            .justify_center()
            .child(bar)
            .into_any_element()
    }

    /// Under the toolbar while a drawing tool is picked, or a layer: its
    /// size and colour.
    fn style_bar(&self, ed: &Editor, cx: &mut Context<Self>) -> Option<AnyElement> {
        let style = ed.shown_style()?;
        let mut bar = ui::bar().relative().h(px(34.)).occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for size in 0..3u8 {
            bar = bar.child(
                size_dot(size, style.size == size)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.with_editor(cx, |ed| ed.restyle(|s| s.size = size)))),
            );
        }
        bar = bar.child(sep());
        for (i, color) in COLORS.into_iter().enumerate() {
            bar = bar.child(
                swatch(i, color, style.color == color)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.with_editor(cx, |ed| ed.restyle(|s| s.color = color)))),
            );
        }
        Some(
            div()
                .absolute()
                .top(px(TOOLBAR + 8.))
                .left(px(self.left()))
                .right(px(self.right()))
                .flex()
                .justify_center()
                .child(bar)
                .into_any_element(),
        )
    }

    /// The layers, on the right: the newest on top, the picture at the
    /// bottom, and buttons to reorder or delete the picked one.
    fn layers(&self, ed: &Editor, cx: &mut Context<Self>) -> AnyElement {
        let n = ed.doc.shapes.len();
        let picked = ed.layer;
        let top = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(LAYERS_TOP))
            .pl(px(12.))
            .pr(px(6.))
            .border_b_1()
            .border_color(rgba(LINE))
            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Layers"))
            .child(
                icon_button("layer-up", IconName::ArrowUp, false, picked.is_some_and(|i| i + 1 < n))
                    .size(px(24.))
                    .tooltip(tip("Bring forward (⌘])"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.with_editor(cx, |ed| ed.reorder_layer(1)))),
            )
            .child(
                icon_button("layer-down", IconName::ArrowDown, false, picked.is_some_and(|i| i > 0))
                    .size(px(24.))
                    .tooltip(tip("Send backward (⌘[)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.with_editor(cx, |ed| ed.reorder_layer(-1)))),
            )
            .child(
                icon_button("layer-delete", IconName::Trash, false, picked.is_some())
                    .size(px(24.))
                    .tooltip(tip("Delete the layer (⌫)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.with_editor(cx, Editor::remove_layer))),
            )
            .child(
                icon_button("layers-close", IconName::X, false, true)
                    .size(px(24.))
                    .tooltip(tip("Hide the layers (⌘L)"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_layers(cx))),
            );
        let mut list = div().id("layers").flex_1().min_h_0().overflow_y_scroll().p(px(6.)).flex().flex_col().gap(px(1.));
        for i in (0..n).rev() {
            list = list.child(self.layer_row(ed, i, cx));
        }
        div()
            .absolute()
            .top(px(FLOAT))
            .right(px(FLOAT))
            .bottom(px(FLOAT))
            .w(px(LAYERS))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .overflow_hidden()
            .bg(rgba(ui::BAR_BG))
            .border_1()
            .border_color(rgba(ui::BAR_LINE))
            .text_size(px(13.))
            .occlude()
            .child(top)
            .child(list)
            .into_any_element()
    }

    fn layer_row(&self, ed: &Editor, i: usize, cx: &mut Context<Self>) -> AnyElement {
        let shape = &ed.doc.shapes[i];
        let hidden = ed.doc.is_hidden(i);
        let picked = ed.layer == Some(i);
        let glyph = match shape {
            Shape::Erase { .. } => IconName::Eraser,
            _ => shape.tool().map_or(IconName::Image, |t| tool_label(t).0),
        };
        let name = match shape {
            Shape::Text { .. } if !shape.name().is_empty() => format!("“{}”", shape.name()),
            Shape::Text { .. } => "Text".to_owned(),
            _ => shape.name(),
        };
        let dot = shape
            .style()
            .map(|st| div().flex_none().size(px(10.)).rounded_full().bg(hsla(st.color)).border_1().border_color(rgba(0xffffff40)));
        let eye = icon_button(("layer-eye", i), if hidden { IconName::EyeOff } else { IconName::Eye }, false, true)
            .size(px(24.))
            .tooltip(tip(if hidden { "Show" } else { "Hide" }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.with_editor(cx, |ed| ed.toggle_hidden(i));
            }));
        div()
            .id(("layer", i))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(LAYER_ROW))
            .pl(px(8.))
            .pr(px(4.))
            .rounded(px(6.))
            .cursor_pointer()
            .map(|d| if picked { d.bg(white()).text_color(black()) } else { d.hover(|s| s.bg(rgba(0xffffff12))) })
            .when(hidden && !picked, |d| d.text_color(rgb(DIM)))
            .child(div().flex_none().when(hidden, |d| d.opacity(0.5)).child(icon(glyph, 14.)))
            .child(div().flex_1().min_w_0().truncate().when(hidden, |d| d.opacity(0.6)).child(name))
            .children(dot)
            .child(eye)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.pick_layer(Some(i), window, cx)))
            .into_any_element()
    }
}

/// A frame on screen, in window points, and what it is for.
#[derive(Clone, Copy)]
enum FrameLook {
    /// The area to crop to, if one is picked yet.
    Crop(Option<(Point<Pixels>, Point<Pixels>)>),
    Select((Point<Pixels>, Point<Pixels>)),
    Pasted((Point<Pixels>, Point<Pixels>)),
}

/// Paint a frame with its handles. A crop dims the picture outside it,
/// and shows the room it adds past the picture as blank paper.
fn paint_frame(picture: Bounds<Pixels>, look: FrameLook, window: &mut Window) {
    let rect = |x0: Pixels, y0: Pixels, x1: Pixels, y1: Pixels| Bounds::from_corners(point(x0, y0), point(x1, y1));
    let (lo, hi, dashed) = match look {
        FrameLook::Crop(None) => return window.paint_quad(fill(picture, hsla([0, 0, 0, 0x8c]))),
        FrameLook::Crop(Some((lo, hi))) => {
            let dim = hsla([0, 0, 0, 0x8c]);
            let (l, t) = (picture.origin.x, picture.origin.y);
            let (r, b) = (l + picture.size.width, t + picture.size.height);
            // The picture outside the area, in four bands.
            let bands = [
                rect(l, t, r, lo.y.clamp(t, b)),
                rect(l, hi.y.clamp(t, b), r, b),
                rect(l, lo.y.clamp(t, b), lo.x.clamp(l, r), hi.y.clamp(t, b)),
                rect(hi.x.clamp(l, r), lo.y.clamp(t, b), r, hi.y.clamp(t, b)),
            ];
            // The area outside the picture, likewise.
            let room = [
                rect(lo.x, lo.y, hi.x, t.clamp(lo.y, hi.y)),
                rect(lo.x, b.clamp(lo.y, hi.y), hi.x, hi.y),
                rect(lo.x, t.clamp(lo.y, hi.y), l.clamp(lo.x, hi.x), b.clamp(lo.y, hi.y)),
                rect(r.clamp(lo.x, hi.x), t.clamp(lo.y, hi.y), hi.x, b.clamp(lo.y, hi.y)),
            ];
            for band in bands.into_iter().filter(|r| r.size.width > px(0.) && r.size.height > px(0.)) {
                window.paint_quad(fill(band, dim));
            }
            for band in room.into_iter().filter(|r| r.size.width > px(0.) && r.size.height > px(0.)) {
                window.paint_quad(fill(band, hsla(raster::PAPER)));
            }
            (lo, hi, false)
        },
        FrameLook::Select((lo, hi)) => (lo, hi, true),
        FrameLook::Pasted((lo, hi)) => (lo, hi, false),
    };
    let edge = rect(lo.x, lo.y, hi.x, hi.y);
    if dashed {
        window.paint_quad(outline(edge, hsla([0, 0, 0, 0xff]), BorderStyle::Solid));
        window.paint_quad(outline(edge, white(), BorderStyle::Dashed));
    } else {
        window.paint_quad(outline(edge, white(), BorderStyle::Solid));
    }
    let lo_p = from_point(lo);
    let hi_p = from_point(hi);
    for (_, at) in model::handles(lo_p, hi_p) {
        let b = Bounds::new(to_point(at - p(HANDLE / 2., HANDLE / 2.)), size(px(HANDLE), px(HANDLE)));
        window.paint_quad(quad(b, px(1.), white(), px(1.), hsla([0, 0, 0, 0xb0]), BorderStyle::Solid));
    }
}

/// A dashed box around the picked layer, in window points.
fn paint_outline(lo: Point<Pixels>, hi: Point<Pixels>, window: &mut Window) {
    let edge = Bounds::from_corners(lo, hi);
    window.paint_quad(outline(edge, hsla([0, 0, 0, 0xff]), BorderStyle::Solid));
    window.paint_quad(outline(edge, white(), BorderStyle::Dashed));
}

/// The stage behind the picture: a faint grid of dots.
fn paint_dots(b: Bounds<Pixels>, window: &mut Window) {
    const STEP: f32 = 20.;
    const DOT: f32 = 1.5;
    let color = hsla([0xff, 0xff, 0xff, 0x1c]);
    let (x0, y0) = (b.origin.x.as_f32(), b.origin.y.as_f32());
    let (w, h) = (b.size.width.as_f32(), b.size.height.as_f32());
    let mut y = STEP / 2.;
    while y < h {
        let mut x = STEP / 2.;
        while x < w {
            let at = point(px(x0 + x - DOT / 2.), px(y0 + y - DOT / 2.));
            window.paint_quad(fill(Bounds::new(at, size(px(DOT), px(DOT))), color).corner_radii(px(DOT / 2.)));
            x += STEP;
        }
        y += STEP;
    }
}

/// The pointer over a frame's `grip`.
fn grip_cursor(grip: Grip, dragging: bool) -> CursorStyle {
    match grip {
        Grip::Move if dragging => CursorStyle::ClosedHand,
        Grip::Move => CursorStyle::OpenHand,
        Grip::N | Grip::S => CursorStyle::ResizeUpDown,
        Grip::E | Grip::W => CursorStyle::ResizeLeftRight,
        Grip::NW | Grip::SE => CursorStyle::ResizeUpLeftDownRight,
        Grip::NE | Grip::SW => CursorStyle::ResizeUpRightDownLeft,
    }
}

fn empty(text: &str) -> AnyElement {
    div().pt(px(80.)).flex().justify_center().text_color(rgb(DIM)).text_size(px(15.)).child(text.to_owned()).into_any_element()
}

//! What the user sets: the capture shortcuts and where captures are saved,
//! kept as JSON in `~/Library/Application Support/dev.gannet/settings.json`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::store;

/// Modifier keys, in the order macOS shows them: ⌃⌥⇧⌘.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub command: bool,
}

impl Mods {
    pub const CONTROL_SHIFT: Mods = Mods { control: true, alt: false, shift: true, command: false };
}

/// A key: its name (the W3C `code` name `global-hotkey` parses), its macOS
/// virtual key code, and how it is shown.
struct KeyDef {
    name: &'static str,
    mac: u16,
    label: &'static str,
}

const fn k(name: &'static str, mac: u16, label: &'static str) -> KeyDef {
    KeyDef { name, mac, label }
}

/// The keys a shortcut can end in. Key codes are the ANSI layout positions
/// (Carbon's kVK_* values), so a shortcut stays on the same physical key
/// whatever the keyboard layout.
const KEYS: &[KeyDef] = &[
    k("KeyA", 0x00, "A"),
    k("KeyS", 0x01, "S"),
    k("KeyD", 0x02, "D"),
    k("KeyF", 0x03, "F"),
    k("KeyH", 0x04, "H"),
    k("KeyG", 0x05, "G"),
    k("KeyZ", 0x06, "Z"),
    k("KeyX", 0x07, "X"),
    k("KeyC", 0x08, "C"),
    k("KeyV", 0x09, "V"),
    k("KeyB", 0x0b, "B"),
    k("KeyQ", 0x0c, "Q"),
    k("KeyW", 0x0d, "W"),
    k("KeyE", 0x0e, "E"),
    k("KeyR", 0x0f, "R"),
    k("KeyY", 0x10, "Y"),
    k("KeyT", 0x11, "T"),
    k("Digit1", 0x12, "1"),
    k("Digit2", 0x13, "2"),
    k("Digit3", 0x14, "3"),
    k("Digit4", 0x15, "4"),
    k("Digit6", 0x16, "6"),
    k("Digit5", 0x17, "5"),
    k("Equal", 0x18, "="),
    k("Digit9", 0x19, "9"),
    k("Digit7", 0x1a, "7"),
    k("Minus", 0x1b, "-"),
    k("Digit8", 0x1c, "8"),
    k("Digit0", 0x1d, "0"),
    k("BracketRight", 0x1e, "]"),
    k("KeyO", 0x1f, "O"),
    k("KeyU", 0x20, "U"),
    k("BracketLeft", 0x21, "["),
    k("KeyI", 0x22, "I"),
    k("KeyP", 0x23, "P"),
    k("Enter", 0x24, "↩"),
    k("KeyL", 0x25, "L"),
    k("KeyJ", 0x26, "J"),
    k("Quote", 0x27, "'"),
    k("KeyK", 0x28, "K"),
    k("Semicolon", 0x29, ";"),
    k("Backslash", 0x2a, "\\"),
    k("Comma", 0x2b, ","),
    k("Slash", 0x2c, "/"),
    k("KeyN", 0x2d, "N"),
    k("KeyM", 0x2e, "M"),
    k("Period", 0x2f, "."),
    k("Tab", 0x30, "⇥"),
    k("Space", 0x31, "Space"),
    k("Backquote", 0x32, "`"),
    k("Backspace", 0x33, "⌫"),
    k("Escape", 0x35, "⎋"),
    k("F17", 0x40, "F17"),
    k("NumpadDecimal", 0x41, "Num ."),
    k("NumpadMultiply", 0x43, "Num *"),
    k("NumpadAdd", 0x45, "Num +"),
    k("NumpadDivide", 0x4b, "Num /"),
    k("NumpadEnter", 0x4c, "Num ↩"),
    k("NumpadSubtract", 0x4e, "Num -"),
    k("F18", 0x4f, "F18"),
    k("F19", 0x50, "F19"),
    k("NumpadEqual", 0x51, "Num ="),
    k("Numpad0", 0x52, "Num 0"),
    k("Numpad1", 0x53, "Num 1"),
    k("Numpad2", 0x54, "Num 2"),
    k("Numpad3", 0x55, "Num 3"),
    k("Numpad4", 0x56, "Num 4"),
    k("Numpad5", 0x57, "Num 5"),
    k("Numpad6", 0x58, "Num 6"),
    k("Numpad7", 0x59, "Num 7"),
    k("F20", 0x5a, "F20"),
    k("Numpad8", 0x5b, "Num 8"),
    k("Numpad9", 0x5c, "Num 9"),
    k("F5", 0x60, "F5"),
    k("F6", 0x61, "F6"),
    k("F7", 0x62, "F7"),
    k("F3", 0x63, "F3"),
    k("F8", 0x64, "F8"),
    k("F9", 0x65, "F9"),
    k("F11", 0x67, "F11"),
    k("F13", 0x69, "F13"),
    k("F16", 0x6a, "F16"),
    k("F14", 0x6b, "F14"),
    k("F10", 0x6d, "F10"),
    k("F12", 0x6f, "F12"),
    k("F15", 0x71, "F15"),
    k("Home", 0x73, "↖"),
    k("PageUp", 0x74, "⇞"),
    k("Delete", 0x75, "⌦"),
    k("F4", 0x76, "F4"),
    k("End", 0x77, "↘"),
    k("F2", 0x78, "F2"),
    k("PageDown", 0x79, "⇟"),
    k("F1", 0x7a, "F1"),
    k("ArrowLeft", 0x7b, "←"),
    k("ArrowRight", 0x7c, "→"),
    k("ArrowDown", 0x7d, "↓"),
    k("ArrowUp", 0x7e, "↑"),
];

fn key_by_name(name: &str) -> Option<&'static KeyDef> {
    KEYS.iter().find(|d| d.name.eq_ignore_ascii_case(name))
}

/// A global shortcut: modifiers and one key. Written as `global-hotkey`
/// parses it, e.g. `control+shift+Digit4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub mods: Mods,
    key: &'static str,
}

impl Shortcut {
    /// `key` is a key name such as `Digit4` or `KeyA`.
    pub fn new(mods: Mods, key: &str) -> Option<Shortcut> {
        key_by_name(key).map(|d| Shortcut { mods, key: d.name })
    }

    /// The shortcut a macOS key press makes (its virtual key code).
    pub fn from_mac(mods: Mods, key_code: u16) -> Option<Shortcut> {
        KEYS.iter().find(|d| d.mac == key_code).map(|d| Shortcut { mods, key: d.name })
    }

    /// The key's name, e.g. `Digit4`.
    pub fn key(&self) -> &'static str {
        self.key
    }

    fn is_function_key(&self) -> bool {
        self.key.strip_prefix('F').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    }

    /// Why this can't be a global shortcut, if it can't: it would take a
    /// key away from typing.
    pub fn problem(&self) -> Option<&'static str> {
        let m = self.mods;
        if m.control || m.alt || m.command || self.is_function_key() { None } else { Some("Add ⌃, ⌥ or ⌘ (F-keys can go alone)") }
    }

    /// The macOS screenshot shortcut this is, if it is one.
    pub fn system_clash(&self) -> Option<&'static str> {
        let cmd_shift = Mods { control: false, alt: false, shift: true, command: true };
        if self.mods != cmd_shift {
            return None;
        }
        match self.key {
            "Digit3" | "Digit4" | "Digit5" | "Digit6" => Some("macOS takes this for its own screenshots"),
            _ => None,
        }
    }

    /// As menus show it, e.g. `⌃⇧4`.
    pub fn label(&self) -> String {
        let m = self.mods;
        let mut s = String::new();
        for (on, sym) in [(m.control, '⌃'), (m.alt, '⌥'), (m.shift, '⇧'), (m.command, '⌘')] {
            if on {
                s.push(sym);
            }
        }
        s.push_str(key_by_name(self.key).map_or(self.key, |d| d.label));
        s
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.mods;
        for (on, name) in [(m.control, "control"), (m.alt, "alt"), (m.shift, "shift"), (m.command, "super")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(self.key)
    }
}

impl FromStr for Shortcut {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mods = Mods::default();
        let mut parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let key = parts.pop().filter(|k| !k.is_empty()).ok_or_else(|| format!("no key in {s:?}"))?;
        for p in parts {
            match p.to_ascii_lowercase().as_str() {
                "control" | "ctrl" => mods.control = true,
                "alt" | "option" => mods.alt = true,
                "shift" => mods.shift = true,
                "super" | "command" | "cmd" => mods.command = true,
                other => return Err(format!("unknown modifier {other:?} in {s:?}")),
            }
        }
        Shortcut::new(mods, key).ok_or_else(|| format!("unknown key {key:?} in {s:?}"))
    }
}

impl Serialize for Shortcut {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Shortcut {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// The captures that have a shortcut.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Region,
    Window,
    Screen,
    Scroll,
}

impl Action {
    pub const ALL: [Action; 4] = [Action::Region, Action::Window, Action::Screen, Action::Scroll];

    pub fn title(self) -> &'static str {
        match self {
            Action::Region => "Capture Region",
            Action::Window => "Capture Window",
            Action::Screen => "Capture Full Screen",
            Action::Scroll => "Capture Scrolling Region",
        }
    }

    /// Gannet's own shortcut for it: the system's ⌘⇧ ones with ⌃ for ⌘.
    pub fn default_shortcut(self) -> Shortcut {
        let key = match self {
            Action::Region => "Digit4",
            Action::Window => "Digit5",
            Action::Screen => "Digit3",
            Action::Scroll => "Digit6",
        };
        Shortcut { mods: Mods::CONTROL_SHIFT, key }
    }
}

fn region_default() -> Option<Shortcut> {
    Some(Action::Region.default_shortcut())
}
fn window_default() -> Option<Shortcut> {
    Some(Action::Window.default_shortcut())
}
fn screen_default() -> Option<Shortcut> {
    Some(Action::Screen.default_shortcut())
}
fn scroll_default() -> Option<Shortcut> {
    Some(Action::Scroll.default_shortcut())
}

/// Each capture's shortcut; `None` (JSON `null`) is none at all, and a
/// missing entry is the default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shortcuts {
    #[serde(default = "region_default")]
    pub region: Option<Shortcut>,
    #[serde(default = "window_default")]
    pub window: Option<Shortcut>,
    #[serde(default = "screen_default")]
    pub screen: Option<Shortcut>,
    #[serde(default = "scroll_default")]
    pub scroll: Option<Shortcut>,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Shortcuts { region: region_default(), window: window_default(), screen: screen_default(), scroll: scroll_default() }
    }
}

impl Shortcuts {
    pub fn get(&self, a: Action) -> Option<Shortcut> {
        match a {
            Action::Region => self.region,
            Action::Window => self.window,
            Action::Screen => self.screen,
            Action::Scroll => self.scroll,
        }
    }

    pub fn set(&mut self, a: Action, s: Option<Shortcut>) {
        match a {
            Action::Region => self.region = s,
            Action::Window => self.window = s,
            Action::Screen => self.screen = s,
            Action::Scroll => self.scroll = s,
        }
    }

    /// The other capture that already has `s`, if one does.
    pub fn taken_by(&self, s: Shortcut, except: Action) -> Option<Action> {
        Action::ALL.into_iter().find(|&a| a != except && self.get(a) == Some(s))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub shortcuts: Shortcuts,
    /// Where captures are saved; `None` is `~/Pictures/Gannet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_dir: Option<PathBuf>,
    /// The capture toolbar is on.
    #[serde(default = "yes")]
    pub toolbar: bool,
    /// Where the capture toolbar is, when on.
    #[serde(default)]
    pub toolbar_place: ToolbarPlace,
    /// Captures older than this many days leave the library; `None` keeps
    /// them until deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_days: Option<u32>,
    /// A capture opens in the library's editor once taken.
    #[serde(default)]
    pub open_editor: bool,
    /// The editor shows its list of layers.
    #[serde(default = "yes")]
    pub layers: bool,
    /// Captures in the trash longer than this many days are deleted for
    /// good; `None` keeps them until the trash is emptied.
    #[serde(default = "trash_default")]
    pub trash_days: Option<u32>,
}

/// Where the capture toolbar is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolbarPlace {
    /// Floating above every window, where it was dragged.
    Floating,
    /// Under the menu bar icon, shown by clicking the icon.
    #[default]
    MenuBar,
}

/// The choices for how long captures are kept, in days.
pub const KEEP_CHOICES: [Option<u32>; 5] = [None, Some(90), Some(30), Some(7), Some(1)];

fn yes() -> bool {
    true
}

fn trash_default() -> Option<u32> {
    Some(30)
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shortcuts: Shortcuts::default(),
            save_dir: None,
            toolbar: true,
            toolbar_place: ToolbarPlace::MenuBar,
            keep_days: None,
            open_editor: false,
            layers: true,
            trash_days: trash_default(),
        }
    }
}

impl Settings {
    /// Where captures go.
    pub fn captures_dir(&self) -> PathBuf {
        self.save_dir.clone().unwrap_or_else(store::captures_dir)
    }

    /// The settings in `path`: the defaults if there is no file, and an
    /// error if it can't be read. Values it can't read are left at their
    /// defaults, keeping the others, and the file is copied beside itself
    /// (`.bad`) before it is saved over.
    pub fn load(path: &Path) -> (Settings, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => (s, None),
                Err(e) => {
                    let mut bad = path.as_os_str().to_owned();
                    bad.push(".bad");
                    let _ = std::fs::copy(path, bad);
                    let kept = serde_json::from_str(&text).ok().and_then(Settings::lenient).unwrap_or_default();
                    (kept, Some(format!("{}: {e}", path.display())))
                },
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Settings::default(), None),
            Err(e) => (Settings::default(), Some(format!("{}: {e}", path.display()))),
        }
    }

    /// The settings in `file`, each value that can't be read left at its
    /// default; `None` if `file` isn't an object.
    fn lenient(file: serde_json::Value) -> Option<Settings> {
        let mut kept = serde_json::to_value(Settings::default()).ok()?;
        merge(&mut kept, &[], file)?;
        serde_json::from_value(kept).ok()
    }

    /// Write to `path`, through a temporary file so a crash never leaves
    /// half a file; the user's alone.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            store::private_dir(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        store::write_private(path, (text + "\n").as_bytes())
    }
}

/// Put each value of object `from` into the object at `at` in `root`
/// where `Settings` can still be read with it, going into objects (the
/// shortcuts) one value at a time.
fn merge(root: &mut serde_json::Value, at: &[String], from: serde_json::Value) -> Option<()> {
    let serde_json::Value::Object(from) = from else { return None };
    for (key, value) in from {
        let old = object_at(root, at)?.insert(key.clone(), value.clone());
        if serde_json::from_value::<Settings>(root.clone()).is_ok() {
            continue;
        }
        let obj = object_at(root, at)?;
        match old {
            Some(old) => obj.insert(key.clone(), old),
            None => obj.remove(&key),
        };
        if value.is_object() && obj.get(&key).is_some_and(|v| v.is_object()) {
            merge(root, &[at, &[key]].concat(), value);
        }
    }
    Some(())
}

fn object_at<'a>(root: &'a mut serde_json::Value, at: &[String]) -> Option<&'a mut serde_json::Map<String, serde_json::Value>> {
    at.iter().try_fold(root, |v, k| v.get_mut(k))?.as_object_mut()
}

/// `~/Library/Application Support/dev.gannet/settings.json`.
pub fn settings_path() -> PathBuf {
    store::home().join("Library/Application Support/dev.gannet/settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(s: &str) -> Shortcut {
        s.parse().unwrap()
    }

    #[test]
    fn round_trips_as_text() {
        let s = sc("control+shift+Digit4");
        assert_eq!(s, Action::Region.default_shortcut());
        assert_eq!(s.to_string(), "control+shift+Digit4");
        assert_eq!(sc("Cmd+Option+keya").to_string(), "alt+super+KeyA");
        assert!("control+Nope".parse::<Shortcut>().is_err());
        assert!("hyper+KeyA".parse::<Shortcut>().is_err());
        assert!("control+".parse::<Shortcut>().is_err());
    }

    #[test]
    fn labels_in_mac_order() {
        assert_eq!(sc("super+shift+alt+control+Digit4").label(), "⌃⌥⇧⌘4");
        assert_eq!(sc("alt+ArrowUp").label(), "⌥↑");
        assert_eq!(sc("F5").label(), "F5");
    }

    #[test]
    fn from_mac_key_codes() {
        assert_eq!(Shortcut::from_mac(Mods::CONTROL_SHIFT, 0x15), Some(sc("control+shift+Digit4")));
        assert_eq!(Shortcut::from_mac(Mods::default(), 0x7a).map(|s| s.key()), Some("F1"));
        assert_eq!(Shortcut::from_mac(Mods::default(), 0x3f), None, "fn is no key");
        let mut codes: Vec<u16> = KEYS.iter().map(|d| d.mac).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), KEYS.len(), "each key code once");
    }

    #[test]
    fn needs_a_real_modifier() {
        assert!(sc("KeyA").problem().is_some());
        assert!(sc("shift+KeyA").problem().is_some());
        assert!(sc("alt+KeyA").problem().is_none());
        assert!(sc("F12").problem().is_none());
        assert!(sc("shift+F1").problem().is_none());
        assert!(sc("super+shift+Digit4").system_clash().is_some());
        assert!(sc("control+shift+Digit4").system_clash().is_none());
    }

    #[test]
    fn finds_shortcuts_taken() {
        let mut s = Shortcuts::default();
        let region = s.region.unwrap();
        assert_eq!(s.taken_by(region, Action::Region), None);
        assert_eq!(s.taken_by(region, Action::Window), Some(Action::Region));
        s.set(Action::Region, None);
        assert_eq!(s.taken_by(region, Action::Window), None);
    }

    #[test]
    fn missing_is_default_null_is_none() {
        let s: Settings = serde_json::from_str(r#"{"shortcuts":{"window":null,"screen":"alt+F3"}}"#).unwrap();
        assert_eq!(s.shortcuts.region, Some(Action::Region.default_shortcut()));
        assert_eq!(s.shortcuts.window, None);
        assert_eq!(s.shortcuts.screen, Some(sc("alt+F3")));
        assert_eq!(s.save_dir, None);
        assert!(s.toolbar, "the toolbar is on unless turned off");
        assert_eq!(s.keep_days, None, "captures are kept until deleted");
        assert!(!s.open_editor);
        assert!(!serde_json::from_str::<Settings>(r#"{"toolbar":false}"#).unwrap().toolbar);
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap(), Settings::default());
    }

    #[test]
    fn a_bad_value_keeps_the_others() {
        let dir = std::env::temp_dir().join(format!("gannet-settings-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let text = r#"{"save_dir":"/tmp/caps","keep_days":7,"trash_days":"lots","shortcuts":{"region":"Cmd+Shift+Nope","window":null}}"#;
        std::fs::write(&path, text).unwrap();
        let (s, err) = Settings::load(&path);
        assert!(err.is_some());
        assert_eq!(s.save_dir, Some(PathBuf::from("/tmp/caps")));
        assert_eq!(s.keep_days, Some(7));
        assert_eq!(s.trash_days, Settings::default().trash_days);
        assert_eq!(s.shortcuts.region, Shortcuts::default().region);
        assert_eq!(s.shortcuts.window, None);
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bad")).unwrap(), text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toolbar_place_defaults_to_menu_bar() {
        let s: Settings = serde_json::from_str(r#"{"toolbar":true}"#).unwrap();
        assert_eq!(s.toolbar_place, ToolbarPlace::MenuBar);
        let s: Settings = serde_json::from_str(r#"{"toolbar_place":"floating"}"#).unwrap();
        assert_eq!(s.toolbar_place, ToolbarPlace::Floating);
    }

    #[test]
    fn saves_and_loads() {
        let dir = std::env::temp_dir().join(format!("gannet-settings-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        assert_eq!(Settings::load(&path), (Settings::default(), None));
        let mut s = Settings { save_dir: Some(PathBuf::from("/tmp/caps")), ..Settings::default() };
        s.shortcuts.set(Action::Window, None);
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), (s.clone(), None));
        assert_eq!(s.captures_dir(), PathBuf::from("/tmp/caps"));
        std::fs::write(&path, "{ not json").unwrap();
        let (loaded, err) = Settings::load(&path);
        assert_eq!(loaded, Settings::default());
        assert!(err.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

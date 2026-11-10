//! The capture library: every capture kept encrypted on disk until the user
//! exports it. Each capture is two files, the image and a small thumbnail
//! for the library's grid, both AES-256-GCM with a key the app keeps in
//! the Keychain. A file is bound to its capture's id and to what it is
//! (image, thumbnail…), so files can't be swapped between captures, or a
//! thumbnail put in place of an image, unnoticed.
//!
//! A capture edited keeps, beside it, its original (`.gorig`), the steps
//! done to it (`.gedit`, see [`crate::edits`]) and the images pasted in
//! (`.<n>.gpaste`), until it is deleted, so every step can be undone even
//! after the app quits. The `.gcap` is then what the steps make of the
//! original, for the grid, copying and exporting.
//!
//! File layout: `GNT2`, a 12-byte nonce, then the ciphertext of the
//! capture's scale (f32, little endian: pixels per point) and its PNG.
//! Files from before 0.3 start `GNT1`: their image and thumbnail are bound
//! to the bare id. They are still read, and written anew as `GNT2`.
//!
//! The library's folder is the user's alone (0700), and so is each file
//! in it (0600).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use image::RgbaImage;

use crate::edits::{Edits, Images};
use crate::folders::Folders;
use crate::store;
use crate::trash::{Trash, Trashed};

const MAGIC: &[u8; 4] = b"GNT2";
/// Files sealed before each kind of file had its own binding.
const MAGIC_V1: &[u8; 4] = b"GNT1";
const NONCE: usize = 12;
const IMAGE_EXT: &str = "gcap";
const THUMB_EXT: &str = "gthumb";
const ORIGINAL_EXT: &str = "gorig";
const EDITS_EXT: &str = "gedit";
const PASTE_EXT: &str = "gpaste";
/// The folders captures are sorted into, sealed like a capture.
const FOLDERS_FILE: &str = "folders.gmeta";
const FOLDERS_ID: &str = "folders";
/// The captures in the trash, sealed likewise.
const TRASH_FILE: &str = "trash.gmeta";
const TRASH_ID: &str = "trash";
/// Held while the trash file is read and written again.
static TRASH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// A thumbnail's longest side, in pixels.
pub const THUMB: u32 = 480;

/// `~/Library/Application Support/dev.gannet/library`.
pub fn library_dir() -> PathBuf {
    store::home().join("Library/Application Support/dev.gannet/library")
}

/// A fresh random key, for the first start.
pub fn new_key() -> [u8; 32] {
    Aes256Gcm::generate_key(OsRng).into()
}

/// The key kept in the file `path`, made there (readable only by the user)
/// the first time. For development builds, which can't use the Keychain.
pub fn file_key(path: &Path) -> Result<[u8; 32], String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    match std::fs::read(path) {
        Ok(bytes) => return bytes.try_into().map_err(|_| format!("{} is damaged", path.display())),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(format!("{}: {e}", path.display())),
        Err(_) => {},
    }
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| e.to_string())?;
    }
    let key = new_key();
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| e.to_string())?;
    f.write_all(&key).map_err(|e| e.to_string())?;
    Ok(key)
}

/// One capture in the library.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Entry {
    /// Its file name without the extension: `<ms since 1970>-<n>`.
    pub id: String,
    /// When it was taken, in ms since 1970.
    pub taken_ms: i64,
}

impl Entry {
    pub fn taken(&self) -> DateTime<Local> {
        Local.timestamp_millis_opt(self.taken_ms).single().unwrap_or_else(Local::now)
    }
}

/// A capture read back: its pixels, and its pixels per point.
pub struct Loaded {
    pub image: RgbaImage,
    pub scale: f32,
}

/// A capture to edit: as it was taken, and what was done to it since.
pub struct Editable {
    pub original: Loaded,
    pub edits: Edits,
    pub images: Images,
}

pub struct Vault {
    dir: PathBuf,
    cipher: Aes256Gcm,
}

impl Vault {
    pub fn new(dir: PathBuf, key: &[u8; 32]) -> Vault {
        Vault { dir, cipher: Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key)) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Keep `img` (with `scale` pixels per point) as a new capture.
    pub fn add(&self, img: &RgbaImage, scale: f32) -> Result<Entry, String> {
        self.add_encoded(img, &store::png_bytes(img)?, scale)
    }

    /// Keep `img` as a new capture, `png` being it already encoded (so a
    /// capture also put on the clipboard is encoded once).
    pub fn add_encoded(&self, img: &RgbaImage, png: &[u8], scale: f32) -> Result<Entry, String> {
        static N: AtomicU32 = AtomicU32::new(0);
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let entry = Entry { id: format!("{ms}-{}", N.fetch_add(1, Ordering::Relaxed)), taken_ms: ms };
        self.write(&entry.id, img, Some(png), scale)?;
        Ok(entry)
    }

    /// Put `img` in place of capture `id` (after it was edited).
    pub fn replace(&self, id: &str, img: &RgbaImage, scale: f32) -> Result<(), String> {
        self.write(id, img, None, scale)
    }

    /// Capture `id` as taken, and the steps done to it.
    pub fn load_editable(&self, id: &str) -> Result<Editable, String> {
        let orig = self.path(id, ORIGINAL_EXT);
        let original = if orig.exists() { self.open_from(&orig, &image_aad(id), id)? } else { self.load(id)? };
        let edits = match std::fs::read(self.path(id, EDITS_EXT)) {
            Ok(bytes) => Edits::from_json(&self.open(&format!("{id}/edits"), &bytes)?)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Edits::default(),
            Err(e) => return Err(e.to_string()),
        };
        let mut images = Images::new();
        for n in edits.images() {
            let aad = format!("{id}/paste/{n}");
            match self.open_from(&self.paste_path(id, n), &aad, &aad) {
                Ok(l) => {
                    images.insert(n, std::sync::Arc::new(l.image));
                },
                Err(e) => log::warn!("pasted image {n} of {id}: {e}"),
            }
        }
        Ok(Editable { original, edits, images })
    }

    /// Keep `edits` as what was done to capture `id`, with the pasted
    /// `images` they use, and `img`, what they make of it, in its place
    /// (`png`: it encoded, if it is already). The original is kept the
    /// first time.
    #[allow(clippy::too_many_arguments)]
    pub fn save_edits(
        &self,
        id: &str,
        edits: &Edits,
        images: &Images,
        img: &RgbaImage,
        png: Option<&[u8]>,
        scale: f32,
    ) -> Result<(), String> {
        let orig = self.path(id, ORIGINAL_EXT);
        if !orig.exists() && !edits.is_empty() {
            // Copied, not moved: the capture is never missing its file.
            // The original is the image as it was, bound alike.
            let bytes = std::fs::read(self.path(id, IMAGE_EXT)).map_err(|e| e.to_string())?;
            store::write_private(&orig, &bytes)?;
        }
        let used: Vec<u32> = edits.images().collect();
        for &n in &used {
            let path = self.paste_path(id, n);
            if let (false, Some(pasted)) = (path.exists(), images.get(&n)) {
                self.seal_to(&path, &format!("{id}/paste/{n}"), pasted, None, scale)?;
            }
        }
        for n in self.paste_numbers(id).into_iter().filter(|n| !used.contains(n)) {
            let _ = std::fs::remove_file(self.paste_path(id, n));
        }
        let edits_path = self.path(id, EDITS_EXT);
        if edits.is_empty() {
            let _ = std::fs::remove_file(&edits_path);
        } else {
            let bytes = self.seal(&format!("{id}/edits"), &edits.to_json())?;
            self.ensure_dir()?;
            store::write_private(&edits_path, &bytes)?;
        }
        self.write(id, img, png, scale)?;
        if edits.is_empty() {
            let _ = std::fs::remove_file(&orig);
        }
        Ok(())
    }

    /// The numbers of the images pasted into capture `id`.
    fn paste_numbers(&self, id: &str) -> Vec<u32> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let prefix = format!("{id}.");
        let suffix = format!(".{PASTE_EXT}");
        dir.flatten().filter_map(|e| e.file_name().to_str()?.strip_prefix(&prefix)?.strip_suffix(&suffix)?.parse().ok()).collect()
    }

    fn paste_path(&self, id: &str, n: u32) -> PathBuf {
        self.dir.join(format!("{id}.{n}.{PASTE_EXT}"))
    }

    /// The image and thumbnail of capture `id`; `png` is `img` encoded,
    /// if it already is.
    fn write(&self, id: &str, img: &RgbaImage, png: Option<&[u8]>, scale: f32) -> Result<(), String> {
        let thumb = thumbnail(img);
        self.seal_to(&self.path(id, THUMB_EXT), &thumb_aad(id), &thumb, None, scale)?;
        self.seal_to(&self.path(id, IMAGE_EXT), &image_aad(id), img, png, scale)
    }

    /// Make the library's folder if it isn't there, the user's alone
    /// (the files are sealed anyway, but their names tell when each
    /// capture was taken).
    fn ensure_dir(&self) -> Result<(), String> {
        store::private_dir(&self.dir)
    }

    /// The captures, newest first. Reads no file, only their names.
    pub fn list(&self) -> Vec<Entry> {
        let Ok(dir) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<Entry> = dir
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                if path.extension()? != IMAGE_EXT {
                    return None;
                }
                parse_id(path.file_stem()?.to_str()?)
            })
            .collect();
        out.sort_by(|a, b| b.taken_ms.cmp(&a.taken_ms).then_with(|| b.id.cmp(&a.id)));
        out
    }

    pub fn load(&self, id: &str) -> Result<Loaded, String> {
        self.open_from(&self.path(id, IMAGE_EXT), &image_aad(id), id)
    }

    pub fn thumb(&self, id: &str) -> Result<Loaded, String> {
        self.open_from(&self.path(id, THUMB_EXT), &thumb_aad(id), id)
    }

    /// The folders captures are sorted into; none before the first.
    pub fn folders(&self) -> Result<Folders, String> {
        self.read_meta(FOLDERS_FILE, FOLDERS_ID)?.map_or(Ok(Folders::default()), |b| Folders::from_json(&b))
    }

    pub fn save_folders(&self, folders: &Folders) -> Result<(), String> {
        self.write_meta(FOLDERS_FILE, FOLDERS_ID, &folders.to_json())
    }

    /// The captures in the trash.
    pub fn trash(&self) -> Result<Trash, String> {
        self.read_meta(TRASH_FILE, TRASH_ID)?.map_or(Ok(Trash::default()), |b| Trash::from_json(&b))
    }

    /// Change the trash with `f`, kept on disk if it says so.
    fn change_trash<R>(&self, f: impl FnOnce(&mut Trash) -> (R, bool)) -> Result<R, String> {
        let _held = TRASH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut trash = self.trash()?;
        let (r, changed) = f(&mut trash);
        if changed {
            self.write_meta(TRASH_FILE, TRASH_ID, &trash.to_json())?;
        }
        Ok(r)
    }

    /// Put captures in the trash at `now_ms`, each with the folder it was in.
    pub fn to_trash(&self, items: &[(String, Option<String>)], now_ms: i64) -> Result<(), String> {
        self.change_trash(|t| ((), items.iter().fold(false, |ch, (id, folder)| t.add(id, now_ms, folder.clone()) | ch)))
    }

    /// Take captures `ids` out of the trash, to be shown again; what they were.
    pub fn restore(&self, ids: &[String]) -> Result<Vec<Trashed>, String> {
        self.change_trash(|t| {
            let back: Vec<Trashed> = ids.iter().filter_map(|id| t.take(id)).collect();
            let changed = !back.is_empty();
            (back, changed)
        })
    }

    /// Delete captures `ids` in the trash for good.
    pub fn delete_for_good(&self, ids: &[String]) -> Result<(), String> {
        ids.iter().try_for_each(|id| self.delete(id))?;
        self.change_trash(|t| ((), ids.iter().fold(false, |ch, id| t.take(id).is_some() | ch)))
    }

    /// Delete for good what has been in the trash more than `max_age_ms`
    /// before `now_ms`; how many.
    pub fn empty_trash(&self, max_age_ms: i64, now_ms: i64) -> Result<usize, String> {
        let old = self.trash()?.expired(max_age_ms, now_ms);
        self.delete_for_good(&old)?;
        Ok(old.len())
    }

    /// A small sealed file of the library's own, if there is one.
    fn read_meta(&self, file: &str, id: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(self.dir.join(file)) {
            Ok(bytes) => self.open(id, &bytes).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write_meta(&self, file: &str, id: &str, plain: &[u8]) -> Result<(), String> {
        self.ensure_dir()?;
        let bytes = self.seal(id, plain)?;
        store::write_private(&self.dir.join(file), &bytes)
    }

    /// Delete capture `id` for good; one already gone is no error.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        if !self.path(id, IMAGE_EXT).exists() {
            return Ok(());
        }
        for n in self.paste_numbers(id) {
            let _ = std::fs::remove_file(self.paste_path(id, n));
        }
        let _ = std::fs::remove_file(self.path(id, EDITS_EXT));
        let _ = std::fs::remove_file(self.path(id, ORIGINAL_EXT));
        let _ = std::fs::remove_file(self.path(id, THUMB_EXT));
        std::fs::remove_file(self.path(id, IMAGE_EXT)).map_err(|e| e.to_string())
    }

    /// Delete the captures taken more than `max_age_ms` before `now_ms`;
    /// how many went.
    pub fn prune(&self, max_age_ms: i64, now_ms: i64) -> usize {
        self.list().into_iter().filter(|e| now_ms - e.taken_ms > max_age_ms).filter(|e| self.delete(&e.id).is_ok()).count()
    }

    fn path(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Seal `img` (or `png`, it encoded already) into `path`, bound to `aad`.
    fn seal_to(&self, path: &Path, aad: &str, img: &RgbaImage, png: Option<&[u8]>, scale: f32) -> Result<(), String> {
        let mut plain = scale.to_le_bytes().to_vec();
        match png {
            Some(png) => plain.extend_from_slice(png),
            None => plain.extend(store::png_bytes(img)?),
        }
        let bytes = self.seal(aad, &plain)?;
        self.ensure_dir()?;
        store::write_private(path, &bytes)
    }

    /// The capture in `path`, bound to `aad`; a file from before 0.3 is
    /// bound to `v1_aad`.
    fn open_from(&self, path: &Path, aad: &str, v1_aad: &str) -> Result<Loaded, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let plain = if bytes.starts_with(MAGIC_V1) { self.open(v1_aad, &bytes)? } else { self.open(aad, &bytes)? };
        let (scale, png) = plain.split_at_checked(4).ok_or("capture too short")?;
        let scale = f32::from_le_bytes(scale.try_into().map_err(|_| "capture too short")?);
        let image = store::decode(png).map_err(|e| format!("can't read the capture: {e}"))?;
        Ok(Loaded { image, scale: if scale.is_finite() && scale > 0. { scale } else { 1. } })
    }

    fn seal(&self, id: &str, plain: &[u8]) -> Result<Vec<u8>, String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = self.cipher.encrypt(&nonce, Payload { msg: plain, aad: id.as_bytes() }).map_err(|_| "can't encrypt the capture")?;
        let mut out = Vec::with_capacity(MAGIC.len() + NONCE + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend(ct);
        Ok(out)
    }

    fn open(&self, id: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
        let rest = bytes.strip_prefix(MAGIC).or_else(|| bytes.strip_prefix(MAGIC_V1)).ok_or("not a Gannet capture")?;
        let (nonce, ct) = rest.split_at_checked(NONCE).ok_or("capture too short")?;
        self.cipher
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: id.as_bytes() })
            .map_err(|_| "can't decrypt the capture (wrong key or damaged file)".to_owned())
    }
}

/// A file in `dir` for exporting `entry`, named by when it was taken and
/// not yet there: `Gannet 2026-11-09 at 13.38.08.png`, then ` 2`, ` 3`…
pub fn export_path(dir: &Path, entry: &Entry) -> PathBuf {
    let stem = entry.taken().format("Gannet %Y-%m-%d at %H.%M.%S").to_string();
    let mut path = dir.join(format!("{stem}.png"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} {n}.png"));
        n += 1;
    }
    path
}

/// What a capture's image (and its original) is bound to.
fn image_aad(id: &str) -> String {
    format!("{id}/image")
}

/// What a capture's thumbnail is bound to.
fn thumb_aad(id: &str) -> String {
    format!("{id}/thumb")
}

fn parse_id(id: &str) -> Option<Entry> {
    let (ms, n) = id.split_once('-')?;
    n.parse::<u32>().ok()?;
    Some(Entry { id: id.to_owned(), taken_ms: ms.parse().ok()? })
}

/// `img` scaled down to fit `THUMB` (never up).
pub fn thumbnail(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let k = (THUMB as f32 / w.max(h).max(1) as f32).min(1.);
    if k >= 1. {
        return img.clone();
    }
    let (tw, th) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    image::imageops::thumbnail(img, tw, th)
}

/// The captures in days, newest first, each with its heading ("Today",
/// "Yesterday", or the date) as seen on `today`.
pub fn by_day<Tz: TimeZone>(entries: &[Entry], today: NaiveDate, tz: &Tz) -> Vec<(String, Vec<Entry>)> {
    let mut out: Vec<(NaiveDate, Vec<Entry>)> = Vec::new();
    for e in entries {
        let day = tz.timestamp_millis_opt(e.taken_ms).single().map(|t| t.date_naive()).unwrap_or(today);
        match out.last_mut() {
            Some((d, list)) if *d == day => list.push(e.clone()),
            _ => out.push((day, vec![e.clone()])),
        }
    }
    out.into_iter().map(|(day, list)| (day_title(day, today), list)).collect()
}

fn day_title(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        "Today".to_owned()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".to_owned()
    } else if day.year() == today.year() {
        day.format("%A, %-d %B").to_string()
    } else {
        day.format("%A, %-d %B %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn vault(name: &str) -> Vault {
        let dir = std::env::temp_dir().join(format!("gannet-vault-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Vault::new(dir, &[7; 32])
    }

    #[test]
    fn the_trash_keeps_captures_until_emptied() {
        let v = vault("trash");
        let img = RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        let a = v.add(&img, 1.).unwrap();
        let b = v.add(&img, 1.).unwrap();
        v.to_trash(&[(a.id.clone(), Some("Work".into())), (b.id.clone(), None)], 1000).unwrap();
        assert_eq!(v.trash().unwrap().items.len(), 2);
        assert_eq!(v.list().len(), 2, "their files stay");
        let back = v.restore(std::slice::from_ref(&a.id)).unwrap();
        assert_eq!(back[0].folder.as_deref(), Some("Work"));
        assert!(!v.trash().unwrap().contains(&a.id));
        assert_eq!(v.empty_trash(500, 1200).unwrap(), 0, "not old enough");
        assert_eq!(v.empty_trash(500, 2000).unwrap(), 1);
        assert_eq!(v.list().len(), 1, "gone for good");
        assert!(v.trash().unwrap().items.is_empty());
    }

    #[test]
    fn folders_are_sealed_and_read_back() {
        let v = vault("folders");
        assert_eq!(v.folders().unwrap(), Folders::default(), "none before the first");
        let mut f = Folders::default();
        let a = f.add("Work");
        f.put("1-0", Some(a));
        v.save_folders(&f).unwrap();
        assert_eq!(v.folders().unwrap(), f);
        let raw = std::fs::read(v.dir().join(FOLDERS_FILE)).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Work"), "sealed");
        assert!(v.list().is_empty(), "not a capture");
    }

    #[test]
    fn keeps_captures_encrypted() {
        let v = vault("round");
        let img = RgbaImage::from_fn(900, 300, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 9, 255]));
        let e = v.add(&img, 2.).unwrap();
        let back = v.load(&e.id).unwrap();
        assert_eq!(back.image, img);
        assert_eq!(back.scale, 2.);
        assert_eq!(v.thumb(&e.id).unwrap().image.dimensions(), (480, 160));
        // Nothing on disk reads as a PNG.
        let raw = std::fs::read(v.dir().join(format!("{}.gcap", e.id))).unwrap();
        assert!(raw.starts_with(MAGIC));
        assert!(image::load_from_memory(&raw).is_err());
        assert!(!raw.windows(4).any(|w| w == b"IHDR"));
        // Another key can't read it.
        assert!(Vault::new(v.dir().to_owned(), &[8; 32]).load(&e.id).is_err());
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn files_are_bound_to_their_capture() {
        let v = vault("bound");
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let a = v.add(&img, 1.).unwrap();
        let b = v.add(&img, 1.).unwrap();
        std::fs::copy(v.dir().join(format!("{}.gcap", a.id)), v.dir().join(format!("{}.gcap", b.id))).unwrap();
        assert!(v.load(&b.id).is_err());
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn a_thumbnail_cant_stand_in_for_the_image() {
        let v = vault("kinds");
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let a = v.add(&img, 1.).unwrap();
        std::fs::copy(v.dir().join(format!("{}.gthumb", a.id)), v.dir().join(format!("{}.gcap", a.id))).unwrap();
        assert!(v.load(&a.id).is_err());
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn reads_captures_from_before_kinds_were_bound() {
        let v = vault("v1");
        let img = RgbaImage::from_pixel(3, 2, image::Rgba([4, 5, 6, 255]));
        let id = "1794205852021-0";
        // As 0.2 wrote them: `GNT1`, image and thumbnail bound to the bare id.
        let mut plain = 2f32.to_le_bytes().to_vec();
        plain.extend(store::png_bytes(&img).unwrap());
        let mut sealed = v.seal(id, &plain).unwrap();
        sealed[..4].copy_from_slice(MAGIC_V1);
        store::private_dir(v.dir()).unwrap();
        for ext in [IMAGE_EXT, THUMB_EXT] {
            std::fs::write(v.dir().join(format!("{id}.{ext}")), &sealed).unwrap();
        }
        let back = v.load(id).unwrap();
        assert_eq!((back.image, back.scale), (img.clone(), 2.));
        assert_eq!(v.thumb(id).unwrap().image, img);
        assert_eq!(v.load_editable(id).unwrap().original.image, img);
        // Edited, it keeps the old file as its original, still read.
        let mut edits = Edits::default();
        edits.push(crate::edits::Op::Crop { x: 0, y: 0, w: 1, h: 1 });
        let small = RgbaImage::from_pixel(1, 1, image::Rgba([4, 5, 6, 255]));
        v.save_edits(id, &edits, &Images::new(), &small, None, 2.).unwrap();
        assert_eq!(v.load_editable(id).unwrap().original.image, img);
        assert_eq!(v.load(id).unwrap().image, small);
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn the_library_is_the_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let v = vault("perms");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        // Folders saved before any capture still make the folder private.
        v.save_folders(&Folders::default()).unwrap();
        assert_eq!(mode(v.dir()), 0o700);
        let img = RgbaImage::from_pixel(2, 2, image::Rgba([1, 1, 1, 255]));
        let a = v.add(&img, 1.).unwrap();
        for f in std::fs::read_dir(v.dir()).unwrap().flatten() {
            assert_eq!(mode(&f.path()), 0o600, "{:?}", f.file_name());
            assert!(!f.file_name().to_string_lossy().ends_with(".tmp"), "no temporary file left");
        }
        assert!(v.dir().join(format!("{}.gthumb", a.id)).exists());
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn lists_replaces_and_deletes() {
        let v = vault("list");
        assert!(v.list().is_empty());
        let img = RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        let a = v.add(&img, 1.).unwrap();
        let b = v.add(&img, 1.).unwrap();
        assert_eq!(v.list(), vec![b.clone(), a.clone()], "newest first");
        let edited = RgbaImage::from_pixel(5, 5, image::Rgba([9, 9, 9, 255]));
        v.replace(&a.id, &edited, 2.).unwrap();
        assert_eq!(v.load(&a.id).unwrap().image, edited);
        v.delete(&a.id).unwrap();
        assert_eq!(v.list(), vec![b]);
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn edits_keep_the_original_beside_the_capture() {
        use crate::edits::Op;
        let v = vault("edits");
        let px = |c: u8| RgbaImage::from_pixel(4, 4, image::Rgba([c, c, c, 255]));
        let a = v.add(&px(1), 2.).unwrap();
        let other = v.add(&px(50), 1.).unwrap();
        // Never edited: the capture is its own original.
        let e = v.load_editable(&a.id).unwrap();
        assert_eq!((e.original.image.clone(), e.edits.is_empty()), (px(1), true));

        let mut edits = Edits::default();
        edits.push(Op::Paste { image: 0, x: 0, y: 0, w: 2, h: 2 });
        edits.push(Op::Crop { x: 0, y: 0, w: 2, h: 2 });
        edits.undo();
        let mut images = Images::new();
        images.insert(0, std::sync::Arc::new(px(7)));
        images.insert(1, std::sync::Arc::new(px(8)));
        v.save_edits(&a.id, &edits, &images, &px(3), None, 2.).unwrap();
        assert_eq!(v.load(&a.id).unwrap().image, px(3), "the capture is what the edits make");
        assert_eq!(v.thumb(&a.id).unwrap().image, px(3));
        let e = v.load_editable(&a.id).unwrap();
        assert_eq!(e.original.image, px(1));
        assert_eq!(e.original.scale, 2.);
        assert_eq!(e.edits, edits, "with what was undone, to redo");
        assert_eq!(e.images.keys().collect::<Vec<_>>(), vec![&0], "only images the steps use are kept");
        assert_eq!(v.list().len(), 2);
        assert!(v.load_editable(&other.id).unwrap().edits.is_empty());

        // Saved again later: still the first original.
        v.save_edits(&a.id, &edits, &e.images, &px(4), None, 2.).unwrap();
        assert_eq!(v.load_editable(&a.id).unwrap().original.image, px(1));

        // Every step gone: back to a plain capture.
        v.save_edits(&a.id, &Edits::default(), &Images::new(), &px(1), None, 2.).unwrap();
        let names = |v: &Vault| -> Vec<String> {
            let mut n: Vec<_> = std::fs::read_dir(v.dir()).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
            n.sort();
            n
        };
        assert_eq!(names(&v).iter().filter(|n| n.starts_with(&a.id)).count(), 2, "{:?}", names(&v));

        // Deleting takes everything with it.
        v.save_edits(&a.id, &edits, &images, &px(3), None, 2.).unwrap();
        v.delete(&a.id).unwrap();
        assert!(names(&v).iter().all(|n| !n.starts_with(&a.id)), "{:?}", names(&v));
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn groups_by_day() {
        let tz = FixedOffset::east_opt(7 * 3600).unwrap();
        let at = |d: u32, h: u32| tz.with_ymd_and_hms(2026, 11, d, h, 0, 0).unwrap().timestamp_millis();
        let e = |ms: i64| Entry { id: format!("{ms}-0"), taken_ms: ms };
        let entries = vec![e(at(9, 13)), e(at(9, 1)), e(at(8, 23)), e(at(2, 10))];
        let today = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let days = by_day(&entries, today, &tz);
        let titles: Vec<_> = days.iter().map(|(t, l)| (t.as_str(), l.len())).collect();
        assert_eq!(titles, vec![("Today", 2), ("Yesterday", 1), ("Monday, 2 November", 1)]);
        let last_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        assert_eq!(by_day(&entries[3..], last_year, &tz)[0].0, "Monday, 2 November 2026");
    }

    #[test]
    fn file_key_is_made_once() {
        let dir = std::env::temp_dir().join(format!("gannet-key-{}", std::process::id()));
        let path = dir.join("k");
        let a = file_key(&path).unwrap();
        assert_eq!(file_key(&path).unwrap(), a);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        std::fs::write(&path, b"short").unwrap();
        assert!(file_key(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prunes_old_captures() {
        let v = vault("prune");
        let img = RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
        let a = v.add(&img, 1.).unwrap();
        let b = v.add(&img, 1.).unwrap();
        assert_eq!(v.prune(1000, b.taken_ms + 10), 0);
        assert_eq!(v.prune(1000, a.taken_ms + 5000).min(2), 2);
        assert!(v.list().is_empty());
        let _ = std::fs::remove_dir_all(v.dir());
    }

    #[test]
    fn export_names_dont_clash() {
        let dir = std::env::temp_dir().join(format!("gannet-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let e = Entry { id: "0-0".into(), taken_ms: 1_794_206_288_000 };
        let first = export_path(&dir, &e);
        assert!(first.file_name().unwrap().to_str().unwrap().starts_with("Gannet 20"));
        std::fs::write(&first, b"x").unwrap();
        let second = export_path(&dir, &e);
        assert!(second.to_str().unwrap().ends_with(" 2.png"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_parse() {
        assert_eq!(parse_id("1794205852021-2").map(|e| e.taken_ms), Some(1794205852021));
        assert!(parse_id("nope").is_none());
        assert!(parse_id("12-x").is_none());
    }
}

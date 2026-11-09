//! Where captures go: the user's captures folder (kept), and a cache folder
//! for the raw grabs `screencapture` writes (deleted right after reading).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use image::RgbaImage;

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// Saved captures: `~/Pictures/Gannet`.
pub fn captures_dir() -> PathBuf {
    home().join("Pictures").join("Gannet")
}

/// Raw grabs on their way in: `~/Library/Caches/dev.gannet/grabs`.
pub fn grabs_dir() -> PathBuf {
    home().join("Library/Caches/dev.gannet/grabs")
}

/// A file name no other capture in this process or earlier has. Never a dot
/// file: `screencapture` won't write those (and still exits 0).
pub fn unique_name(prefix: &str) -> String {
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("{prefix}-{ms}-{}.png", N.fetch_add(1, Ordering::Relaxed))
}

/// Write `img` as a new PNG in `dir`.
pub fn save(img: &RgbaImage, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(unique_name("Gannet"));
    img.save_with_format(&path, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `img` encoded as PNG.
pub fn png_bytes(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(buf.into_inner())
}

/// Delete PNGs in `dir` older than `max_age`.
pub fn cleanup(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > max_age));
        if old && e.path().extension().is_some_and(|x| x == "png") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_not_hidden() {
        assert!(!unique_name("grab").starts_with('.'));
        assert!(unique_name("grab").ends_with(".png"));
        assert_ne!(unique_name("a"), unique_name("a"));
    }

    #[test]
    fn saves_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("gannet-store-test-{}", std::process::id()));
        let img = RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        let a = save(&img, &dir).unwrap();
        let b = save(&img, &dir).unwrap();
        assert_ne!(a, b);
        assert_eq!(image::open(&a).unwrap().to_rgba8(), img);
        cleanup(&dir, Duration::from_secs(3600));
        assert!(a.exists(), "new files stay");
        std::thread::sleep(Duration::from_millis(20));
        cleanup(&dir, Duration::from_millis(1));
        assert!(!a.exists() && !b.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encodes_png() {
        let img = RgbaImage::from_pixel(2, 2, image::Rgba([9, 8, 7, 255]));
        let bytes = png_bytes(&img).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), img);
    }
}

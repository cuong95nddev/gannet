# Roadmap

Gannet follows [FastStone Capture](https://www.faststone.org/FSCaptureDetail.htm)'s
feature set, on macOS. ✅ done, the rest in rough order.

## Capture

- ✅ Rectangular region, on a frozen screen, with a size readout
- ✅ Window (macOS's window picker), without its shadow
- ✅ Full screen, every display
- Window / object under the pointer, highlighted before the click (own
  picker in the overlay, through ScreenCaptureKit's window list)
- Active window, no click
- Fixed-size region (a size you set, moved into place)
- Freehand region (lasso)
- Scrolling window: a long page or document, stitched as it scrolls
- Repeat the last capture (same region)
- Delayed capture (timer, to open menus first)
- Include the mouse pointer (optional)
- A magnifier by the pointer while picking, and arrow keys to move by a
  pixel

## Edit and mark up

- ✅ Rectangle, ellipse, line, arrow, pen, text, numbered markers, mosaic,
  colours and sizes, undo/redo
- A separate editor window for a capture after it is taken (Tern's
  `shot/editor.rs` is the starting point)
- Highlighter, blur (not only mosaic), spotlight (dim all but an area)
- Crop, resize, rotate, flip
- Border, shadow, torn edge, rounded corners, caption and watermark
- Combine captures side by side or stacked

## Output

- ✅ Clipboard, `~/Pictures/Gannet`, Save as…
- Choose what happens after a capture: editor, clipboard, file, several
- File name templates (date, time, counter) and folder per setting
- PNG, JPEG (quality), WebP, PDF
- Open with another app, share sheet, upload (FTP/S3/custom command)
- A history of recent captures (Tern's tray in `shot/tray.rs`)

## Screen tools

- Colour picker (hex/RGB, copied)
- Screen ruler (pixels, horizontal/vertical)
- Screen crosshair
- Screen magnifier

## Screen recording

- Region, window or full screen to MP4 (ScreenCaptureKit + AVFoundation)
- System audio and microphone
- Show clicks and keystrokes
- Trim, and export as GIF

## App

- Settings window (shortcuts, output, file names)
- Shortcuts you choose
- Floating capture toolbar (FastStone's capture panel)
- Launch at login
- Signed, notarized `.app`, Homebrew cask, updates

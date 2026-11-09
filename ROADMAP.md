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
- ✅ Scrolling region (⌃⇧6): pick a region, scroll what is in it, and the
  frames are stitched into one tall picture (fixed bars at its top and
  bottom kept once)
- Scrolling on its own (auto-scroll, needs Accessibility access)
- Repeat the last capture (same region)
- Delayed capture (timer, to open menus first)
- Include the mouse pointer (optional)
- A magnifier by the pointer while picking, and arrow keys to move by a
  pixel

## Edit and mark up

- ✅ Rectangle, ellipse, line, arrow, pen, text, numbered markers, mosaic,
  colours and sizes, undo/redo
- ✅ Non-destructive: the original and every step are kept (encrypted)
  until the capture is deleted, saved as you go, so each step can be
  undone or redone after a restart
- ✅ An editor for a capture after it is taken, in the library window:
  markup, zoom (⌘+/⌘−/⌘0/⌘1, pinch, ⌘-scroll), pan, ← → between captures,
  and the captures listed beside it to jump to one
- ✅ Layers: each shape listed on the editor's right (⌘L shows or hides
  the list), picked there or on the picture to move, recolour, resize its
  stroke, hide, reorder (⌘[ ⌘]) or delete (⌫), every step undoable
- Redact: a solid box for secrets. Mosaic only averages 8 px blocks, which
  depixelation tools can sometimes read back, so it is cosmetic, not redaction
- Highlighter, blur (not only mosaic), spotlight (dim all but an area)
- ✅ Crop (applied as it is let go); drag the picture's edge out to add
  blank room, in to crop
- ✅ Select an area to copy (⌘C), paste an image (⌘V) to move and resize
  before it is laid down
- ✅ The picture and pasted images are layers on a canvas: picked, they
  move and resize inside it (a crop only changes the canvas)
- Rotate, flip
- Border, shadow, torn edge, rounded corners, caption and watermark
- Combine captures side by side or stacked

## Output

- ✅ Clipboard, and an encrypted library (AES-256-GCM, key in the
  Keychain) that keeps every capture until it is exported
- ✅ Export one capture (Save as…) or several at once to a folder
- ✅ Delete captures older than a set number of days
- ✅ Open the editor after a capture (optional)
- Choose what else happens after a capture: file, several
- File name templates (date, time, counter) and folder per setting
- PNG, JPEG (quality), WebP, PDF
- Open with another app, share sheet, upload (FTP/S3/custom command)
- ✅ A library of captures by day, several picked at once (⌘-click, ⇧-click)
- ✅ Folders: a capture goes in its day's folder; make, rename and delete
  folders and drag captures between them, in a tree
- ✅ Trash: deleting moves captures there, to put back in their folder or
  delete for good; emptied of what is older than a set number of days

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

- ✅ Settings window with a sidebar: shortcuts you choose (recorded,
  checked for clashes), the folder captures are saved to, permissions
- Settings for the output format and file names
- ✅ Floating capture toolbar (FastStone's capture panel), hidden while
  capturing, where it was left
- Launch at login
- ✅ Signed with a Developer ID, notarized DMG from the Release workflow
- ✅ Website (`site/`, GitHub Pages)
- App icon, Homebrew cask, updates

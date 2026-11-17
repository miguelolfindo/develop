# Develop user guide

Develop is a fast, free raw photo editor for Windows and macOS. Every adjustment runs on the
graphics card, so sliders respond instantly even on 61-megapixel files, and the sliders are
tuned against Lightroom Classic so a given value looks much as it does there. This guide
covers everything the app does, in the order you would use it.

**Contents**

1. [Installing and updating](#1-installing-and-updating)
2. [The window](#2-the-window)
3. [Finding and organising photos](#3-finding-and-organising-photos)
4. [Looking at a photo](#4-looking-at-a-photo)
5. [Editing](#5-editing)
   - [Presets](#presets) · [Versions](#versions) · [Profile and treatment](#profile-and-treatment)
     · [Crop and straighten](#crop-and-straighten) · [White balance](#white-balance)
     · [Tone](#tone) · [Presence](#presence) · [Tone curve](#tone-curve)
     · [Color mixer](#color-mixer) · [Black and white mix](#black-and-white-mix)
     · [Detail](#detail) · [Lens correction](#lens-correction) · [Retouch](#retouch)
6. [Undo, reset, copy and paste](#6-undo-reset-copy-and-paste)
7. [Exporting](#7-exporting)
8. [Converting to DNG](#8-converting-to-dng)
9. [Where your edits are kept](#9-where-your-edits-are-kept)
10. [AI assistants](#10-ai-assistants)
11. [Keyboard and mouse](#11-keyboard-and-mouse)
12. [Supported files and cameras](#12-supported-files-and-cameras)
13. [Command line](#13-command-line)
14. [Troubleshooting](#14-troubleshooting)
15. [Privacy](#15-privacy)

---

## 1. Installing and updating

### Windows

Download `develop_x.y.z_x64-setup.exe` from the [Releases](https://github.com/miguelolfindo/develop/releases) page and run it.
It installs for your user account only (into `%LOCALAPPDATA%\Develop`), so no administrator
rights are needed. Installing a newer version over an older one keeps your settings and edits.

### macOS

Apple silicon Macs with macOS 13.4 or later. Download `Develop_x.y.z_aarch64.dmg`, open it, and
drag Develop into Applications. Develop is not signed with an Apple developer certificate, so
macOS blocks the first launch: open **System Settings › Privacy & Security**, scroll to
**Security** and click **Open Anyway**, then open Develop again. The README's
[Installing on macOS](../README.md#installing-on-macos) section covers the "damaged" message and
other first-launch problems.

### Updates

When Develop starts it checks GitHub for a newer release (in the background, never blocking
you). If there is one, a dialog shows what is new, with three choices:

- **Download and install**: downloads the installer with a progress bar, then closes Develop
  and installs the update (on a Mac it replaces the app and opens the new version).
- **Later**: asks again next time.
- **Skip this version**: does not offer this version again (a later one will be offered).

An update is only offered once its installer for your system is ready to download.

### Uninstalling

Windows: **Settings › Apps**, or run `uninstall.exe` in `%LOCALAPPDATA%\Develop`. macOS: drag
Develop from Applications to the Bin. Your photos and their `.dvlp` edit files stay where they
are; the settings folder (see [section 9](#9-where-your-edits-are-kept)) can be deleted by hand.

---

## 2. The window

```
┌──────────────────────────────── top bar ────────────────────────────────┐
│ folder panel │            photo (viewer)              │   inspector     │
│  (Folders)   │                                        │  (edit panel)   │
│              │  zoom · A/B Split · Clipping           │  [ Export… ]    │
├──────────────┴──────────────── filmstrip ─────────────┴─────────────────┤
```

- **Top bar:** buttons to show or hide the folder panel and the inspector (also `[` and `]`), the
  open photo's file name, camera, size and bit depth, and which graphics system Develop is
  using (Vulkan, DX12, Metal, OpenGL or WebGPU).
- **Folder panel** (left): your drives, home folder and network.
- **Viewer** (middle): the photo, with the zoom readout, **A/B Split** and **Clipping** in the
  bottom-left corner.
- **Inspector** (right): the photo's name and type, **Undo**, **Redo** and **Reset All**, the
  camera, size and as-shot white balance (hover for details), then the View, Histogram and
  Vectorscope sections and every editing section. **Export…** sits at the bottom.
- **Filmstrip** (bottom): the photos of the current folder.

Sections collapse and expand by clicking their titles. Double-clicking a slider's name puts it
back to its default.

---

## 3. Finding and organising photos

### Opening photos

- Click a folder in the folder panel to show its photos in the filmstrip, then click a photo.
- **Ctrl+O** (Cmd+O on a Mac) opens a file with the system's file dialog.
- Drag a photo file onto the window.
- When Develop starts it comes back to the folder and photo you had open last time.

### Folder panel

Lists your drives, your home folder (and **Volumes** on a Mac) and **Network**. Click the arrow
next to a folder to open it, click its name to show its photos. **Read again** (the circular
arrow) reads the drives, network and folders again.

**Network** lists the computers and NAS boxes on your network and the folders they share. To
add one that is not listed, type its address in the box at the top of Network (`\\computer\share`
on Windows, `smb://computer/share` on a Mac) and confirm. Windows asks for a user name and
password if the computer needs them; on a Mac, use **Connect…**, which signs in through Finder
and your Keychain. Right-click an added computer and choose **Remove from Network** to remove it.

### Filmstrip

Each card shows the photo, its file type (amber for camera raw files, teal for DNG and pictures),
its star rating, a sliders mark when it has been edited, and a stacked-frames mark with a number
when it has saved versions.

- **Click** a card to open that photo. **Ctrl-click** (Cmd-click) adds or removes photos from the
  selection; **Shift-click** selects everything from the open photo to the one clicked.
- **Left and right arrow keys** move to the previous or next photo.
- **Sort** by Capture Date, File Name or File Type, ascending or descending (the arrow button).
- **Filter** to show only rated photos, a minimum number of stars, or five stars only.
- **Refresh** (the circular arrow) reads the folder again for new or deleted files.
- The header shows the folder, which photo of how many is open, and how many are selected.
- The mouse wheel scrolls the strip sideways.

**Versions in the filmstrip.** A photo with saved [versions](#versions) is shown together with
them in one tinted frame: the photo's own card first, then a smaller card per version, marked
**V1**, **V2**… with the version's name underneath. Each version card shows a thumbnail of that
version (drawn once the photo has been opened; until then the photo's own thumbnail, dimmed).
Click a version card to open the photo at that version; right-click it to **Open this version**
or **Delete this version**.

### Ratings

Press **1** to **5** to give the selected photos (or the open photo) that many stars, **0** to
remove the rating, or use **Rating** in the right-click menu. Ratings are kept in each photo's
`.dvlp` file.

### Right-click menu

Right-click a photo in the filmstrip, or the photo in the viewer:

- **Export…** (or **Export N photos…** when several are selected)
- **Convert to DNG…** and **Convert to DNG, embedding the original…**
- **Copy settings** and **Paste settings** (to N photos)
- **Rating**

When several photos are selected and you right-click one of them, the menu applies to all of them.

---

## 4. Looking at a photo

- **Zoom:** scroll the mouse wheel (or pinch on a trackpad) to zoom around the pointer.
  Double-click to switch between fit and 100%. The **View** section has **Fit**, **100%** and
  **200%** buttons, and the zoom readout in the viewer's corner switches between fit and 100%.
- **Pan:** drag the photo.
- **Rotate:** the two rotate buttons in the View section turn the photo a quarter turn left or
  right. Photos open upright as the camera recorded them.
- **A/B Split:** shows the photo before your edit on the left and after on the right, with a
  divider you drag. In the [Versions](#versions) section, **A/B** shows a saved version as the
  "before" instead.
- **Clipping:** marks blown highlights in red (every channel at maximum) or amber (some
  channels), and crushed blacks in blue.
- **Histogram:** red, green and blue, white where all three overlap.
- **Vectorscope:** colours by hue (angle) and strength (distance from the centre); the amber line
  is skin tone.

---

## 5. Editing

Every edit is non-destructive: the original file is never changed. Edits are saved
automatically, a moment after you stop moving a slider, in a small `.dvlp` file beside the photo
(see [section 9](#9-where-your-edits-are-kept)).

The inspector's sections follow the order most photos are worked on. Slider scales follow
Lightroom: Exposure in stops (-5 to +5), most other sliders -100 to +100 with 0 meaning no
change.

### Presets

Save the current look and reuse it on other photos.

- **Save:** type a name and click **Save**. Tick **Include white balance** if the preset should
  set temperature and tint too (useful for photos shot in the same light). Saving under an
  existing name replaces that preset.
- **Apply:** click a preset's name. It applies to the open photo, or to every selected photo in
  the filmstrip.
- **Delete:** the **×** next to a preset.

A preset carries the profile, tone, presence, tone curve, color mixer or black and white mix,
and detail settings. Crop, rotation, lens correction, retouching and AI denoise stay as they are
on each photo. Presets with white balance are marked **WB** in the list.

### Versions

Keep several versions of one photo's edit and switch between them.

- **Save:** type a name (or leave it empty for "Version 1", "Version 2"…) and click **Save**. A
  version keeps the whole edit, including crop and retouching.
- **Switch:** click a version's name. **Undo** takes you back to the edit you had.
- **A/B:** shows that version as "before" in the A/B split, against your current edit.
- **Update:** replaces the version with the current edit.
- **×:** deletes the version.

Versions are kept in the photo's `.dvlp` file, so they travel with it, and appear in the
[filmstrip](#filmstrip). Versions made by an AI assistant are labelled `AI · ...` and
`Before AI · ...` (see [section 10](#10-ai-assistants)).

### Profile and treatment

- **Treatment:** **Color** or **Black & White**. Black & White turns the photo monochrome,
  replaces the Color Mixer with the [B&W Mix](#black-and-white-mix), and switches off Vibrance
  and Saturation.
- **Profile:** how the camera's colours and base tone are rendered:
  - **Adobe camera profiles** (Adobe Color, Adobe Standard and others) when Lightroom or Camera
    Raw is installed on the computer, including an approximation of Adobe Color.
  - **Standard (built-in):** Develop's own tone curve and colour.
  - **Linear:** no base curve: flat and dark, exactly what the sensor recorded.

  For JPEG, PNG, TIFF and HEIC pictures the profile is fixed ("Picture (sRGB)").

### Crop and straighten

Click **Crop & Straighten** (or press **R**). In crop mode:

- Drag the frame or its edges and corners to crop.
- Drag outside the frame to rotate (straighten); the **Angle** shows the result, up to ±45°. The
  crop shrinks to stay inside the turned photo and grows back when you turn back.
- Press **Enter**, double-click, or click **Done Cropping** (or **R** again) to finish.
- **Reset** clears the crop and the angle.
- **Ratio:** Free, Original, 1:1, 5:4, 4:3, 3:2, 7:5, 16:10, 16:9, 2:1, or one of your own.
  **Swap** turns the ratio between landscape and portrait. Under **New ratio**, enter a width and
  height and a name and click **Save** to keep it in the list; **Delete** removes the selected
  saved ratio.

The viewer shows the cropped size in pixels and the angle while you crop.

### White balance

- **Temp:** colour temperature in kelvin (2000-12000 K). Double-click its name for the
  as-shot value.
- **Pick** (or **W**): the white balance eyedropper. Click something in the photo that should be
  neutral grey or white, and temperature and tint are set to make it so. Esc cancels. Develop
  says so if the spot is too dark or too bright to judge.
- **As Shot:** the camera's own white balance.
- **5500 K:** daylight.
- **Auto:** Develop finds the white balance itself. When there are people in the photo it goes by
  their skin: it finds the faces, judges their skin as if the faces were correctly exposed, and
  cools (or warms) the picture until skin looks natural without turning pink. That is what makes
  firelight, candles and tungsten come out right, where the camera leaves everything orange and
  nothing in the picture is grey. Without faces it makes the grey and white parts neutral, and if
  there is nothing to go by it changes nothing. Only temperature (and tint, from grey areas) is
  changed, never exposure; a note under the button says what it went by, and **Undo** goes back.
- **Tint:** -150 (green) to +150 (magenta), relative to the as-shot white balance.

### Tone

- **Exposure:** overall brightness, in stops.
- **Contrast**
- **Highlights:** brings bright areas down (or up) while keeping their texture.
- **Shadows:** opens dark areas (or deepens them).
- **Whites** and **Blacks:** set the brightest and darkest points.

Highlights, Shadows and Whites adapt to each photo's own brightness, as Lightroom's do.

### Presence

- **Texture:** fine detail and surface texture (negative values soften, for example skin).
- **Clarity:** local contrast in the midtones.
- **Dehaze:** removes (or adds) haze.
- **Vibrance:** strengthens muted colours more than strong ones, and protects skin tones.
- **Saturation:** strengthens or weakens every colour equally (-100 is grey).

### Tone curve

- **The graph:** click the curve to add a point, drag points to shape it, double-click a point to
  remove it (the end points stay). The readout below shows the point's input and output on
  Lightroom's 0-255 scale.
- **Highlights, Lights, Darks, Shadows:** push regions of the tone range up or down. They are
  fitted to Lightroom's own (Darks reaches across most of the range, for example).
- **Reset:** a straight curve and every region at 0.

The curve is applied so colours keep their hue (a sunset stays orange when its highlights come
down). Measured against Lightroom, a curve lands within about 1-2 levels of Lightroom's.

### Color mixer

Hue, saturation and luminance (brightness) for eight colours: red, orange, yellow, green, aqua,
blue, purple and magenta. Choose **Hue**, **Saturation** or **Luminance** at the top; **Reset**
puts that tab's sliders back to 0.

- **Hue** moves a colour towards its neighbour (red towards orange or magenta, for example).
- **Saturation** -100 turns a colour grey at its own brightness; +100 roughly doubles it.
- **Luminance** brightens or darkens a colour without changing its hue.

It works on the colours as you see them on screen, and leaves greys alone. Orange covers skin,
sand and warm light; blue covers most skies; green and yellow cover foliage.

### Black and white mix

Shown instead of the Color Mixer when the treatment is **Black & White**: one slider per colour
sets how bright that colour turns out in grey. For example, lower **Blue** for a darker sky,
raise **Orange** for brighter skin or sand, raise **Green** and **Yellow** for lighter foliage.
**Reset** puts every colour back to 0.

### Detail

- **AI Denoise:** cleans sensor noise with the RawNIND model. It works on raw files (including
  Fujifilm X-Trans) and on phone DNGs; switching it on or off opens the photo again, which takes
  a moment (progress shows in the inspector). On Windows it runs on the graphics card through
  DirectML; on a Mac, on the processor (Bayer raws) or the graphics card (other files).
- **Sharpening:** amount (0-150; 40 by default for raw files).
- **Radius:** the size of the detail sharpened (0.5-3 pixels).
- **Detail:** how much fine detail is sharpened (0-100).
- **Masking:** leaves smooth areas such as sky and skin unsharpened (0-100).

Sharpening is shown as it will look at the current zoom, so fit view looks like a resized export.

### Lens correction

- **Enable Profile Corrections** with a **Source**:
  - **Auto:** a Lensfun profile for the lens when there is one, otherwise the camera's embedded
    corrections if the file has them (Sony).
  - **Lensfun:** a lens profile from the Lensfun database, which comes with Develop. The lens is
    matched automatically; pick another with **Make**, **Model** and **Profile**.
  - **Camera embedded corrections:** the correction data in the raw file.
- **Profile Distortion** and **Profile Vignetting:** how much of the profile's correction is
  applied (100% by default, up to 200%).
- **Constrain Crop:** enlarges just enough that corrections never show empty borders.
- **Distortion:** manual barrel or pincushion correction.
- **Defringe:** **Purple Amount** and **Purple Hue**, **Green Amount** and **Green Hue** remove
  coloured fringes on high-contrast edges.
- **Red / Cyan** and **Blue / Yellow:** correct colour fringes towards the corners (lateral
  chromatic aberration).
- **Vignette:** **Amount** darkens or lightens the corners, **Midpoint** sets how far in it reaches.

For Nikon Z, Panasonic, Olympus / OM System and Fujifilm cameras, distortion correction is
applied automatically even with profile corrections off, as Lightroom does for these cameras.
To add lenses, open the Lensfun folder from the panel, drop Lensfun `.xml` files in, and reload.

### Retouch

Remove dust spots, blemishes and small distractions. Click **Remove Spots** (or press **Q**):

- **Click** a spot to remove it with a circle, or **drag** to paint over a longer one (a wire, a
  branch, a scratch). Develop picks a nearby area to copy from.
- **Mode:** **Heal** blends the copied area into its surroundings; **Clone** copies it exactly.
- Drag either outline to move it (the target or its source). The two are joined by a line when
  selected.
- **Size**, **Feather** and **Opacity** apply to the selected spot, or to new spots when none is
  selected.
- **[** and **]** make the brush (and the selected spot) smaller or bigger; **Shift+[** and
  **Shift+]** change the feather.
- **Delete** (or Backspace) removes the selected spot; **Clear** removes all.
- **H** hides the outlines; **Space**+drag (or the middle mouse button) pans; **Esc** deselects,
  then leaves the tool.

Spots are layered in order: a Heal painted after a Clone copies from, and blends into, the cloned
result, so you can clone out an awkward edge first and heal over it. Healing is worked out across
the whole painted shape, so wide patches blend smoothly. A photo can have up to 64 spots.

---

## 6. Undo, reset, copy and paste

- **Undo** and **Redo**: **Ctrl+Z**, and **Ctrl+Shift+Z** or **Ctrl+Y** (Cmd on a Mac), or the
  buttons at the top of the inspector. Each photo has its own history while Develop is open; one
  slider drag is one step.
- **Reset All:** the whole edit back to Develop's defaults: every slider, white balance (as
  shot), crop and angle, tone curve, color mixer, black and white, sharpening, lens correction and
  retouching. Undo brings it back. Versions and the rating are kept.
- **Copy and paste settings:** **Ctrl+C** copies the open photo's settings (also from the
  right-click menu); **Ctrl+V** pastes them onto the open photo, or onto every selected photo.
  Pasting onto photos that are not open keeps each photo's own crop and rotation. Copying also
  puts a line of text on the system clipboard, which is what makes Ctrl+V reliable on Windows.

---

## 7. Exporting

Click **Export…** at the bottom of the inspector, press **Ctrl+E**, or right-click photos and
choose **Export…**. With several photos selected, each is exported with its own settings into a
folder you choose, with a progress bar; Develop says when it is finished and can show the files.

**Preset:** keep the export settings below under a name (**Save**), pick one, or **Delete** it.

**Format**

| Format | Options |
|---|---|
| JPEG | **Quality** 1-100 |
| PNG | **Bit depth** 8 or 16 bits |
| TIFF | **Bit depth** 8 or 16 bits |
| DNG (original raw) | The original raw data, not a rendered picture: see [section 8](#8-converting-to-dng). **Embed the original raw file** stores the raw file inside the DNG too. |

**Colour space:** sRGB.

**Size:** Original size, or resize by Long edge, Short edge, Width, Height (pixels),
Megapixels, or Percentage. **Don't enlarge** never makes a picture bigger than the original.
**Resolution** (pixels per inch) is a print-size hint written into the file; it does not change
the pixels.

**Metadata**

- **All:** camera, lens, exposure, capture date, artist and copyright. **Remove location** leaves
  the GPS position out.
- **Copyright only:** artist and copyright.
- **None:** no metadata at all.
- **Artist** and **Copyright:** text written into the file (filled in from the camera when it
  recorded them).

TIFF files carry the camera, date, artist and copyright only.

---

## 8. Converting to DNG

DNG is Adobe's open raw format. Converting saves the original raw data, losslessly compressed,
for editing in other apps or for archiving.

- Right-click raw photos and choose **Convert to DNG…** (or **…embedding the original…** to store
  the original raw file inside the DNG as well, about twice the size), or choose **DNG (original
  raw)** as the format in the Export dialog.
- Develop writes the DNG itself. It keeps the camera and lens details (all of the original's
  EXIF), the camera's baseline exposure, and, for cameras whose distortion Lightroom always
  corrects (Nikon Z, Panasonic, Olympus / OM System, Fujifilm), that correction in a standard form
  other DNG readers apply too.
- Develop's edits come along in a `.dvlp` file beside each DNG, so Develop reopens it as you left
  it. Other apps open the DNG unedited.
- The originals are never changed or deleted.

---

## 9. Where your edits are kept

**Beside each photo:** `DSC01234.ARW.dvlp`, a small text file with the photo's edit, rating and
saved versions. Only edited or rated photos get one. Keep it with the photo when you copy or move
it, and the edit comes along. If a folder cannot be written to (a memory card, a read-only disk),
the edit is kept in the settings folder instead.

**Changes made by another program show at once.** While a folder is open, Develop is told when
another program changes one of its `.dvlp` files: an AI assistant editing through Develop's MCP
server, or a second copy of Develop. New ratings and versions appear in the filmstrip, and if the
changed photo is the one you have open, its sliders and picture update and a note says *Edited by
another program*; **Undo** brings back what you had. The newest change wins, so neither program
saves an old copy over the other's. (Develop does not check the folder on a timer for this; the
system tells it. On a shared folder on another computer it checks every few seconds instead.)
Other photos' thumbnails show the camera's preview until you open them again.

**The settings folder:** `%APPDATA%\Develop` on Windows, `~/Library/Application Support/Develop`
on a Mac.

| File | Holds |
|---|---|
| `develop-presets.tsv` | Your develop presets |
| `export-settings.tsv` | Last export settings and export presets |
| `crop-presets.tsv` | Your saved crop ratios |
| `session.tsv` | The folder and photo to reopen |
| `network.tsv` | Computers and shares added to Network |
| `photo-settings.tsv` | Edits for photos in folders that cannot be written to |
| `update-skip.txt` | A version you chose to skip |
| `thumbs/` | Thumbnails of edited photos and of versions |
| `lensfun/` | Your extra Lensfun lens profiles |
| `mcp-http.txt` | The address and token of the HTTP MCP server, when used |

Earlier versions of Develop kept their settings under `SonyRawProcessor`; they are copied over
the first time a newer version starts.

---

## 10. AI assistants

Develop includes an MCP server, so an AI assistant (Claude, Gemini, ChatGPT-based tools, local
models in LM Studio, Cursor, VS Code Copilot, Codex, Continue and others) can help with your
photos: list a folder, read and change edits, measure a photo with numbers (histogram, clipping,
colour cast) or look at a preview, apply presets, rate, save and restore versions, export, and
convert to DNG.

Develop protects your work while an assistant edits:

- Your current edit is kept first as a version named `Before AI · <time>`.
- Each request's changes to a photo go into one version named after the app and what changed,
  for example `AI · Claude: exposure, highlights · 17 Nov 14:32 UTC`. Alternatives the assistant
  makes on request (a black and white take, say) get their own version.
- Versions you made can never be changed or removed by an assistant.

Setup for each AI app, every tool and setting, good practice and privacy notes are in
**[MCP.md](MCP.md)** (including `check_photos`, which lets an assistant measure a whole folder at once; focus checks that measure the full-size photo and show the assistant its sharpest area at 100%; `cull_photos`, which rates a whole shoot by focus, keeping the sharpest frame of each burst; and a skin-tone reading, so an assistant can judge white balance from people's faces in firelight or tungsten, where nothing in the picture is grey), and a ready-made guide to give your assistant is in
**[develop-skill](develop-skill/SKILL.md)**.

---

## 11. Keyboard and mouse

Ctrl means Cmd on a Mac. Keys work when no text box has the keyboard.

| Key | Action |
|---|---|
| Ctrl+O | Open a file |
| ← / → | Previous / next photo in the filmstrip |
| 1-5, 0 | Rate the selected photos (0 removes the rating) |
| Ctrl+C / Ctrl+V | Copy / paste settings |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z or Ctrl+Y | Redo |
| Ctrl+E | Export |
| R | Crop and straighten on / off |
| Enter | Finish cropping |
| W | White balance eyedropper (Esc cancels) |
| Q | Retouch (spot removal) on / off |
| [ / ] | Show / hide the folder panel / inspector; in Retouch, brush smaller / bigger |
| Shift+[ / Shift+] | In Retouch: feather harder / softer |
| Delete, Backspace | In Retouch: remove the selected spot |
| H | In Retouch: hide / show the outlines |
| Space+drag | In Retouch: pan |
| Esc | In Retouch: deselect, then leave the tool |

| Mouse | Action |
|---|---|
| Scroll / pinch | Zoom around the pointer |
| Drag | Pan (in Retouch: paint, or move an outline) |
| Double-click the photo | Fit / 100% (in crop mode: finish) |
| Middle-button drag | Pan (also in Retouch) |
| Double-click a slider's name | Back to its default |
| Right-click | Export, Convert to DNG, Copy / Paste settings, Rating |
| Ctrl-click / Shift-click in the filmstrip | Add to the selection / select a range |

---

## 12. Supported files and cameras

**Raw files:** every format the rawler decoder reads, around 825 camera models: ARW, CR2, CR3,
CRW, CRM, NEF, NRW, RW2, RWL, RAF (including X-Trans), ORF, ORI, PEF, SRW, IIQ, 3FR, FFF, DCR,
DCS, KDC, MEF, MOS, MRW, ERF, X3F, ARI, QTK, RAW and DNG (including phone DNGs).

**Pictures:** JPEG, PNG, TIFF and HEIC, edited in linear light so the same sliders work on them.

**Calibrated against Lightroom:** Sony, Nikon, Canon, Panasonic and Fujifilm. Sliders, white
balance and Lightroom's per-camera adjustments (built-in lens corrections, exposure offsets) were
measured on reference photos from these brands' cameras; on 18 reference photos a slider lands
within about 4 levels (out of 255) of Lightroom on average.

**Supported, not yet calibrated:** every other camera the decoder reads (Olympus / OM System,
Pentax, Leica, Samsung, Hasselblad, Phase One, Sigma, Kodak, Minolta, Epson, phones). They open
and edit normally, with colour from the camera's own data (and Adobe's profile when Lightroom or
Camera Raw is installed); a slider may look a little different from Lightroom.

---

## 13. Command line

For scripting and automation. On Windows the program is
`%LOCALAPPDATA%\Develop\develop.exe`; on a Mac,
`/Applications/Develop.app/Contents/MacOS/develop`.

| Command | Does |
|---|---|
| `develop <photo>` | Open Develop on a photo |
| `develop <photo> --export <out.jpg> [options]` | Export one photo and quit, without opening a window (format from the file extension: `.jpg`, `.png`, `.tif`) |
| `develop <first> [options] --batch-export <folder> <more photos…>` | Export several photos into a folder and quit |
| `develop --to-dng <folder> [--embed] <raw files…>` | Convert raw files to DNG in a folder |
| `develop --mcp` | MCP server on standard input / output (see [MCP.md](MCP.md)) |
| `develop --mcp-http <port> [--mcp-token <token>]` | MCP server over HTTP on this computer |

Export options: `--export-long <pixels>`, `--export-percent <percent>`, `--export-quality <1-100>`,
`--export-bits 16`, `--export-metadata <0 none, 1 copyright, 2 all>`, and `--with-settings 1` to
use the photo's saved edit (without it a command-line export uses Develop's defaults).
Other `--name value` options set sliders for testing.

---

## 14. Troubleshooting

- **A photo opens dark, flat or with odd colours.** Check the **Profile**: Linear is flat and dark
  on purpose; pick an Adobe profile or Standard.
- **Colours differ slightly from Lightroom.** Expected for cameras outside the five calibrated
  brands, and when Adobe's camera profiles are not installed (install Lightroom or Camera Raw to
  get them).
- **AI Denoise is greyed out.** It works on camera raw files and phone DNGs, not on JPEG, PNG,
  TIFF or HEIC. If it says the model files were not found, reinstall Develop.
- **Ctrl+V does nothing.** Copy the settings with Ctrl+C in Develop first (that also puts text on
  the clipboard Ctrl+V needs), or use the right-click menu.
- **Edits did not follow a copied photo.** Copy its `.dvlp` file along with it.
- **A network computer is missing.** Add it by address in Network; on Windows, make sure network
  discovery is on; on a Mac, use Connect….
- **Changes made by an AI assistant do not show.** They normally appear within a second. On some
  network shares Develop is not told about changes and checks every few seconds instead; wait a
  moment, or open the folder again.
- **Graphics problems.** The top bar shows which graphics system Develop uses; updating the
  graphics driver usually helps.
- **macOS says the app is damaged or will not open.** See the README's
  [Installing on macOS](../README.md#installing-on-macos).

---

## 15. Privacy

Develop has no account, no cloud and no telemetry. It works on the files on your computer and
network. It connects to the internet only to check GitHub for updates when it starts. Its own AI
(noise reduction) runs entirely on your computer. An AI assistant connected through MCP sees only
what you point it at; a cloud assistant receives the photo previews it asks for, as anything you
paste into its chat would (see [MCP.md](MCP.md#privacy-and-safety)).

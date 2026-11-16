## Develop 0.3.2

### Retouch: heal and clone
- New **Retouch** section (press **Q**): click a dust spot or blemish to remove it, or
  **paint over** a longer one, such as a wire, a branch or a scratch. Develop picks a nearby
  area to copy from.
- **Heal** blends the copied area into its surroundings, even across wide painted patches;
  **Clone** copies it exactly.
- Spots are layered in order: clone out an awkward edge first, then heal over it, and the
  heal works from the cloned result.
- Drag either outline to move it. `[` `]` change the brush size, Shift+`[` `]` the feather,
  and each spot has its own opacity. Space+drag pans, Delete removes the selected spot, H
  hides the outlines, Esc finishes.
- Spots are saved with the photo, can be undone, and are copied with Copy / Paste settings
  (handy for sensor dust across a series).

### Tone Curve and Color Mixer
- **Tone Curve:** a point curve you edit on the graph (click to add a point, drag it,
  double-click to remove it) and Lightroom's four regions: Highlights, Lights, Darks and
  Shadows. Fitted to Lightroom's own exports: a curve lands within about 1 to 2 levels of
  Lightroom's.
- **Color Mixer:** hue, saturation and luminance for eight colours, fitted to Lightroom's
  exports (close, not yet exact; purple and magenta are not calibrated yet).

### White balance eyedropper
- Click **Pick** in White Balance (or press **W**), then click something on the photo that
  should be neutral grey or white.

### A tidier edit panel
- The sections now follow the order most photos are edited in: Profile, Crop, White
  Balance, Tone, Presence, Tone Curve, Color Mixer, Detail, Lens Correction, Retouch.
- Vibrance and Saturation moved into Presence, as in Lightroom.

### Fixes
- **AI Denoise** no longer turns phone DNGs (such as Samsung Galaxy S24 Ultra Expert RAW)
  reddish, and no longer draws red fringes around bright edges.
- **AI Denoise** on Fujifilm X-Trans raws gave badly wrong colours; it now works properly.

### Installing
- **Windows:** run `develop_0.3.2_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.2_aarch64.dmg`, see below.

#### macOS

**1. Install**

1. Double-click the `.dmg`. A window opens showing Develop.
2. Drag Develop into the Applications folder.
3. Eject the disk image (the eject icon next to it in Finder's sidebar).

**2. First launch (the app is unsigned)**

Develop isn't signed with an Apple developer certificate, so macOS blocks it the first time.

1. Open Applications and double-click Develop. macOS will say it can't be opened, or that
   it can't verify the developer.
2. Click **Done** or **Cancel**.
3. Open **System Settings → Privacy & Security** and scroll down to **Security**. You'll see
   a line saying "Develop" was blocked. Click **Open Anyway**, then enter your password.
4. Double-click Develop again and click **Open** in the dialog.

From then on it opens normally.

**If macOS says "Develop is damaged and can't be opened":** that's the download quarantine
flag, not real damage. Open Terminal and run:

```
xattr -cr /Applications/Develop.app
```

Then open the app again.

**If it bounces in the Dock and quits immediately,** re-sign it locally and try again:

```
codesign --force --deep --sign - /Applications/Develop.app
```

### Known limitations
- The Color Mixer is calibrated on one photo so far; purple and magenta are approximate.
- Heal brings in the colour of whatever surrounds the spot; next to busy detail, clone the
  edge first and heal over it.
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.

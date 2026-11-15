## Develop 0.3.0

### Automatic updates
- When it starts, Develop checks whether a newer version is out. If there is one, a dialog
  shows **what's new** and offers to **download and install** it, or to remind you later
  or skip that version.
  - **Windows:** the installer downloads and runs, and Develop closes so it can update.
  - **macOS:** the new version replaces Develop in Applications and opens by itself.
- From this version on, you won't need to come back here for updates.

### Undo and redo, per photo
- **Undo** and **Redo** buttons beside **Reset All**, and the usual shortcuts:
  **Ctrl+Z** / **Ctrl+Shift+Z** or **Ctrl+Y** on Windows, **Cmd+Z** / **Shift+Cmd+Z** on macOS.
- Every photo keeps its own history while Develop is open, so undoing on one photo never
  touches another.
- One slider drag is one step; sliders, crop, straighten, rotation, profile, lens
  corrections and Reset All can all be undone.

### Easier exporting
- **Right-click a photo** in the filmstrip (or the picture itself) and choose **Export…**.
  With several photos selected it becomes **Export N photos…** and exports them all, each
  with its own edits.
- A **progress window** shows which photo is being exported and how far along it is, with a
  Cancel button.
- When it is done, an **Export complete** window says so, with **Show in folder** to go
  straight to the files. If any photo failed, it says which.

### Also
- The README now explains how Develop compares with Lightroom: free and open source, what
  it does well, and what it does not have.
- The AI noise reduction models are now part of the source code, with their origin and
  license in `models/README.md`.

### Upgrading from 0.1.x?
Version 0.2.0 added Nikon, Canon, Panasonic and Fujifilm raw files (including X-Trans) and
sliders that match Lightroom much more closely; 0.2.1 added network browsing. See the
[0.2.0](https://github.com/miguelolfindo/develop/releases/tag/v0.2.0) and
[0.2.1](https://github.com/miguelolfindo/develop/releases/tag/v0.2.1) release notes.

### Installing
- **Windows:** run `develop_0.3.0_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.0_aarch64.dmg`, see below.

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
- The automatic update needs this version or later: if you have 0.2.1 or older, install
  0.3.0 by hand once.
- Undo history is kept while Develop is open; it starts fresh the next time.
- Dehaze remains the slider furthest from Lightroom.

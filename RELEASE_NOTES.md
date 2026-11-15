## Develop 0.3.1

### Many more cameras
- Develop now opens every raw format its decoder reads: around 825 camera models, adding
  Olympus / OM System, Pentax, Leica, Samsung, Hasselblad, Phase One, Sigma, Kodak, Minolta,
  older Canon and Nikon formats and more.
- **Calibrated against Lightroom:** Sony, Nikon, Canon, Panasonic and Fujifilm. Other cameras
  open and edit normally, with colour from the camera's own data, but have not been compared
  with Lightroom yet, so a slider may look a little different. See the README's Cameras
  section.

### Convert to DNG
- Save the original raw data in Adobe's open **DNG** format, losslessly compressed:
  right-click photos and choose **Convert to DNG…**, or pick **DNG (original raw)** as the
  format in the Export dialog. Works on one photo or a whole selection, with a progress bar.
- Optionally **embed the original raw file** inside the DNG.
- The DNG keeps the camera and lens details (all of the original's EXIF), opens at the same
  brightness as the original, and for cameras whose distortion Lightroom always corrects
  (Nikon Z, Panasonic, Olympus / OM System, Fujifilm) carries that correction in a standard
  form other apps apply too.
- Your Develop edits come along in a `.dvlp` file beside each DNG, so Develop reopens it as
  you left it. Other apps open the DNG unedited.

### Fixes
- Fujifilm X-Trans DNGs opened with wrong colours; they now look right.

### Installing
- **Windows:** run `develop_0.3.1_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.1_aarch64.dmg`, see below.

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
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.
- Copies of Develop older than 0.3.0 cannot update automatically: install this version by
  hand once.

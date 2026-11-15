# Develop

A fast raw photo editor for Windows and macOS. Every adjustment runs on the GPU, so sliders
respond instantly, even on 61-megapixel files. The sliders are tuned against Lightroom
Classic exports of photos from Sony, Nikon, Canon, Panasonic and Fujifilm cameras, so
moving a slider in Develop looks much like moving the same slider in Lightroom.

## Develop and Lightroom

Develop is **free and open-source software** (GNU GPL v3). There is nothing to buy, no
subscription, no account and no cloud. It is an independent project, not affiliated with or
endorsed by Adobe.

**What it is:** a develop-only raw editor. You browse folders on your drives or your network
and edit photos directly; each photo's edits are saved in a small `.dvlp` file beside it, and
the original is never changed. The global sliders are tuned against Lightroom Classic exports,
so a given slider value looks much like it does in Lightroom: on 18 reference photos from
Sony, Nikon, Canon, Panasonic and Fujifilm cameras, a slider lands within about 4 levels (out
of 255) of Lightroom on average.

**What it does well:** fast global editing (every adjustment runs on the GPU), Lightroom-style
Tone, Presence and Color sliders, AI noise reduction, lens corrections, crop and straighten,
undo and redo, copying settings between photos, ratings and filters, and exporting one photo
or a whole selection to JPEG, PNG or TIFF.

**What it does not have (Lightroom does):**
- A catalog: no keywords, collections, face recognition, map or search across your library.
- Local adjustments: no masks, brushes, gradients, or AI subject and sky selection.
- Healing and cloning, and removing objects.
- A tone curve, a colour mixer (HSL) or colour grading panel, and develop presets.
- HDR and panorama merge, printing, mobile apps and cloud sync.

AI is used for one thing only: noise reduction, running on your own computer.

## Features

- **Raw files:** Sony ARW, Nikon NEF, Canon CR2 and CR3, Panasonic RW2, Fujifilm RAF
  (including X-Trans sensors) and DNG, plus JPEG, PNG, TIFF and HEIC.
- **Edit panel:** Crop, Tone, Presence, Color, Detail and Lens Correction.
  - **Tone:** Exposure, Contrast, Highlights, Shadows, Whites, Blacks, on Lightroom's
    scales. Highlights, Shadows and Whites adapt to each photo's own brightness, as
    Lightroom's do.
  - **Presence:** Texture, Clarity, Dehaze, also adapting to the photo.
  - **Color:** white balance (temperature and tint), Vibrance, Saturation.
  - **Detail:** sharpening and **AI noise reduction** (RawNIND model, runs on the GPU
    through DirectML on Windows; works on Bayer raws, X-Trans raws and linear DNGs).
  - **Lens correction:** Lensfun lens profiles (database included), the camera's own
    correction data for Sony lenses, plus manual distortion, defringe and vignette.
    Distortion correction is applied automatically for Nikon Z, Panasonic, Olympus / OM
    System and Fujifilm cameras, as Lightroom does.
- **Camera profiles:** uses the Adobe camera profiles (`.dcp`) installed with Lightroom
  or Camera Raw when they are present, including an approximation of Adobe Color.
- **Filmstrip** with ratings, **histogram and scopes**, **before/after split**, and
  clipping warnings.
- **Network browsing:** the folder tree has a **Network** entry listing the computers and
  NAS boxes on your network and the folders they share; add one by address
  (`\\computer\share` on Windows, `smb://computer/share` on macOS). Windows asks for a
  password when a computer needs one; on macOS, shares are mounted through Finder and
  your Keychain.
- **Non-destructive:** edits are saved in a small `.dvlp` sidecar file next to each photo;
  the original is never changed.
- **Export** to JPEG, PNG or TIFF, with resizing and metadata options, one photo or a
  batch.

## Install

Download the installer for your system from the
[Releases](../../releases) page:

- **Windows:** `develop_x.y.z_x64-setup.exe`. Run it; it installs for your user account
  only, so no administrator rights are needed.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_x.y.z_aarch64.dmg`. See
  [Installing on macOS](#installing-on-macos) below.

### Installing on macOS

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

```bash
xattr -cr /Applications/Develop.app
```

Then open the app again.

**If it bounces in the Dock and quits immediately,** re-sign it locally and try again:

```bash
codesign --force --deep --sign - /Applications/Develop.app
```

## Building from source

Requires [Rust](https://rustup.rs/) (stable).

```bash
cargo build --release
```

The program is `target/release/develop` (`develop.exe` on Windows).

**AI noise reduction** uses the two RawNIND model files in `models/`, which come with the
repository, so a fresh clone has AI denoise for every supported format. See
[models/README.md](models/README.md) for where they come from and their license.

**Installers** are built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager):

```bash
cargo install cargo-packager --locked
```

```bash
cargo packager --release
```

This makes an NSIS installer on Windows and a `.dmg` on macOS, in `dist/`. Pushing a tag
such as `v0.1.3` builds both on GitHub Actions and attaches them to a release.

## Lightroom reference plugin

`lightroom/DevelopReferences.lrplugin` is a Lightroom Classic plugin used to tune
Develop's sliders. For each selected photo it resets the photo, exports a blank
reference JPEG, then one JPEG per slider value (for example `DSC05615-cla+050.jpg`),
using a Lightroom export preset. Add it in Lightroom with **File › Plug-in Manager ›
Add**, then run **Library › Plug-in Extras › Export Reference JPEGs…**.

## License

Copyright (C) 2026 Miguel Olfindo

Develop is free software: you can redistribute it and/or modify it under the terms of
the GNU General Public License as published by the Free Software Foundation, either
version 3 of the License, or (at your option) any later version. It is distributed in the
hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty
of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See [LICENSE](LICENSE) for the
full text.

## Credits

- Raw decoding: [rawler](https://github.com/dnglab/dnglab) (LGPL-2.1).
- Sony embedded lens corrections are read the way
  [darktable](https://github.com/darktable-org/darktable) reads them.
- `patches/objc2` and `patches/objc2-0.3`: modified copies of
  [objc2](https://github.com/madsmtm/objc2), MIT license (see the `LICENSE-MIT` file in
  each).
- Lens profiles: the [Lensfun](https://github.com/lensfun/lensfun) database, CC BY-SA
  3.0 (see `lensfun/README.md`).
- AI noise reduction model: RawNIND (Brummer & De Vleeschouwer), GPL-3.0, as packaged
  by [darktable-ai](https://github.com/darktable-org/darktable-ai).
- User interface: [egui / eframe](https://github.com/emilk/egui); GPU:
  [wgpu](https://github.com/gfx-rs/wgpu); ONNX inference:
  [ort](https://github.com/pykeio/ort).
- Adobe camera profiles are not included; Develop reads them only if Lightroom or Camera
  Raw is installed.

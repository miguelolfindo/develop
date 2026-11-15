# Develop

A fast raw photo editor for Windows and macOS. Every adjustment runs on the GPU, so sliders
respond instantly, even on 61-megapixel files. The sliders are tuned against Lightroom
Classic exports, so moving a slider in Develop looks much like moving the same slider in
Lightroom.

## Features

- **Raw files:** Sony ARW, Nikon NEF and DNG, plus JPEG, PNG, TIFF and HEIC.
- **Edit panel:** Crop, Tone, Presence, Color, Detail and Lens Correction.
  - **Tone:** Exposure, Contrast, Highlights, Shadows, Whites, Blacks. Highlights, Shadows
    and Whites adapt to each photo's brightness, as Lightroom's do.
  - **Presence:** Texture, Clarity, Dehaze.
  - **Color:** white balance (temperature and tint), Vibrance, Saturation.
  - **Detail:** sharpening and **AI noise reduction** (RawNIND model, runs on the GPU
    through DirectML on Windows).
  - **Lens correction:** Lensfun lens profiles (database included), the camera's own
    correction data for Sony lenses, Nikon Z distortion correction applied automatically,
    plus manual distortion, defringe and vignette.
- **Camera profiles:** uses the Adobe camera profiles (`.dcp`) installed with Lightroom
  or Camera Raw when they are present, including an approximation of Adobe Color.
- **Filmstrip** with ratings, **histogram and scopes**, **before/after split**, and
  clipping warnings.
- **Non-destructive:** edits are saved in a small `.dvlp` sidecar file next to each photo;
  the original is never changed.
- **Export** to JPEG, PNG or TIFF, with resizing and metadata options, one photo or a
  batch.

## Install

Download the installer for your system from the
[Releases](../../releases) page:

- **Windows:** `develop_x.y.z_x64-setup.exe`. Run it; it installs for your user account
  only, so no administrator rights are needed.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_x.y.z_aarch64.dmg`. Open it and
  drag Develop to Applications. The app is not notarized, so the first time, right-click
  (or Control-click) Develop in Applications and choose **Open**, then **Open** again.

## Building from source

Requires [Rust](https://rustup.rs/) (stable).

```bash
cargo build --release
```

The program is `target/release/develop` (`develop.exe` on Windows).

**AI noise reduction** needs the RawNIND model files, which are not kept in the
repository. Download `rawdenoise-nind.dtmodel` from the
[darktable-ai releases](https://github.com/darktable-org/darktable-ai/releases) (it is a
zip file), extract `model_bayer.onnx` and `model_linear.onnx`, and save them as
`models/rawnind_bayer.onnx` and `models/rawnind_linear.onnx`. Without them the app works
normally; only the AI denoise switch is unavailable.

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

## Credits and licenses

- Develop is licensed under the **GNU General Public License v3.0 or later**.
- Raw decoding: [rawler](https://github.com/dnglab/dnglab).
- Lens profiles: the [Lensfun](https://github.com/lensfun/lensfun) database, CC BY-SA
  3.0 (see `lensfun/README.md`).
- AI noise reduction model: RawNIND, as packaged by
  [darktable](https://github.com/darktable-org/darktable-ai).
- User interface: [egui / eframe](https://github.com/emilk/egui); GPU:
  [wgpu](https://github.com/gfx-rs/wgpu); ONNX inference:
  [ort](https://github.com/pykeio/ort).
- Adobe camera profiles are not included; Develop reads them only if Lightroom or Camera
  Raw is installed.

# Develop

A fast raw photo editor for Windows and macOS. Every adjustment runs on the GPU, so sliders
respond instantly, even on 61-megapixel files. The sliders are tuned against Lightroom
Classic exports of photos from Sony, Nikon, Canon, Panasonic and Fujifilm cameras, so
moving a slider in Develop looks much like moving the same slider in Lightroom.

## Documentation

- **[User guide](docs/USER-GUIDE.md):** every feature, step by step: browsing, searching,
  keywords, people, sharing a library, moving folders, editing, presets, versions, retouching, exporting, DNG, keyboard shortcuts,
  where edits are kept, and troubleshooting.
- **[AI assistants (MCP)](docs/MCP.md):** connecting an AI app, every tool and setting, good
  practice and privacy; plus a ready-made [assistant guide](docs/develop-skill/SKILL.md).
- **[Release notes](RELEASE_NOTES.md)** for the current version, and the
  [Releases](https://github.com/miguelolfindo/develop/releases) page for downloads and history.

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
Tone, Presence and Color sliders, a tone curve and colour mixer, a white balance eyedropper,
noise reduction for every photo plus AI denoise for raw files, lens corrections, crop and straighten, spot healing and cloning (circles or
painted strokes), black & white, develop presets, saved versions of an edit, undo and redo,
copying settings between photos, ratings and filters, keywords (with suggestions from what is
in each photo), face recognition, search across a library of folders (including by
description), a library shared between computers (a PC and a Mac, say), moving folders safely
with the library following them, exporting one photo or a whole selection to JPEG, PNG or TIFF, and an MCP server so
AI assistants can help with your work.

**What it does not have (Lightroom does):**
- A full catalog: no collections, smart collections, map or import; the library is a list of
  folders, and keywords and names live in each photo's `.dvlp` file.
- Local adjustments: no masks, brushes, gradients, or AI subject and sky selection.
- Content-aware fill and AI object removal: retouching copies from a source you can
  move (heal or clone), it does not invent content.
- A colour grading panel and per-channel (red, green, blue) curves.
- HDR and panorama merge, printing, mobile apps and cloud sync.

Develop's own AI runs entirely on your own computer: noise reduction, face recognition, and
reading what is in your photos for keyword suggestions and search. If you choose to, you can
also connect an AI assistant of your own through the [MCP server](#ai-assistants-mcp).

## Features

- **Raw files:** every format the [rawler](https://github.com/dnglab/dnglab) decoder reads,
  around 825 camera models: ARW, CR2, CR3, CRW, CRM, NEF, NRW, RW2, RWL, RAF (including
  X-Trans), ORF, ORI, PEF, SRW, IIQ, 3FR, FFF, DCR, DCS, KDC, MEF, MOS, MRW, ERF, X3F, ARI,
  QTK, RAW and DNG, plus JPEG, PNG, TIFF and HEIC. See [Cameras](#cameras) for which ones
  are calibrated against Lightroom.
- **Edit panel,** in the order most photos are worked on:
  - **Presets:** save the current look as a preset (optionally with its white balance), and
    apply a preset to the open photo or to every selected photo with one click. A preset
    carries tone, presence, colour, curve, mixer, detail and profile settings; crop,
    rotation, lens correction and retouching stay as they are on each photo.
  - **Versions:** save named versions of a photo's edit, switch between them (Undo goes
    back), update or delete them, and show any version as "before" in the A/B split to
    compare it with the current edit. Versions are kept in the photo's `.dvlp` file, and
    appear in the filmstrip as smaller cards (V1, V2...) grouped with their photo in a
    shared frame; click one to open the photo at that version.
  - **Profile:** **Color** or **Black & White** treatment, and the camera profile (Adobe
    camera profiles when installed, see below, or the built-in one). In Black & White the
    Color Mixer becomes a **B&W Mix** that sets how bright each colour turns out in grey,
    and Vibrance and Saturation are switched off.
  - **Crop:** crop, straighten and aspect ratio.
  - **White Balance:** temperature and tint, an eyedropper (W): click something that
    should be neutral and the white balance is set to make it so, **Auto**, which makes the
    grey parts of the picture neutral, or, when there is almost nothing grey (firelight,
    tungsten), goes by people's skin, and **Skin**, which goes by the people only.
  - **Tone:** Exposure, Contrast, Highlights, Shadows, Whites, Blacks, on Lightroom's
    scales. Highlights, Shadows and Whites adapt to each photo's own brightness, as
    Lightroom's do.
  - **Presence:** Texture, Clarity, Dehaze (also adapting to the photo), Vibrance and
    Saturation.
  - **Tone Curve:** a point curve (click to add a point, drag, double-click to remove) and
    Lightroom's four regions (Highlights, Lights, Darks, Shadows), applied so hues hold.
  - **Color Mixer:** hue, saturation and luminance for eight colours, on the colours as
    displayed. Both are fitted to Lightroom references (one photo so far): the tone curve
    lands within about 1-2 levels of Lightroom's, the Color Mixer within about 6.
  - **Detail:** sharpening; **Noise Reduction** as in Lightroom (Luminance with Detail and
    Contrast, Color with Detail and Smoothness), live on the GPU for every photo, JPEGs
    included; and **AI noise reduction** (RawNIND model, runs on the GPU through DirectML on
    Windows; works on Bayer raws, X-Trans raws and linear DNGs).
  - **Lens correction:** Lensfun lens profiles (database included), the camera's own
    correction data for Sony lenses, plus manual distortion, defringe and vignette.
    Distortion correction is applied automatically for Nikon Z, Panasonic, Olympus / OM
    System and Fujifilm cameras, as Lightroom does.
  - **Retouch** (Q): click a dust spot or blemish to remove it, or paint over a longer
    one (a wire, a branch, a scratch). Develop picks a nearby area to copy from; **Heal**
    blends it into the surroundings, **Clone** copies it as it is. Spots are layered in
    order: a Heal painted after a Clone copies from and blends into the cloned result, so
    you can clone out an awkward edge first and heal over it. Drag either outline to move
    it; `[` `]` change the brush size, Shift+`[` `]` the feather, and each spot has its own
    opacity. Space+drag (or the middle button) pans, Delete removes the selected spot, H
    hides the outlines, Esc finishes. Spots are saved with the photo's other edits, can be
    undone, and are copied with Copy / Paste settings (useful for sensor dust across a
    series).
- **Camera profiles:** uses the Adobe camera profiles (`.dcp`) installed with Lightroom
  or Camera Raw when they are present, including an approximation of Adobe Color.
- **Layout:** tabs for **Folders**, **Faces & Metadata** and the **Inspector**; drag the panels
  and the filmstrip to the size you like; a **grid view** fills the window with thumbnails.
- **EXIF:** when, camera, lens, shutter speed, aperture, ISO, location and more, for the open or
  selected photo, at the top of Faces & Metadata.
- **Keywords and search:** add keywords (Develop suggests them from what is in the photo; you
  click to accept), and search a folder or your whole library by keyword, person, file name or a
  description such as "people eating noodles".
- **Library and people:** add folders to the library; Develop recognises faces across it and
  groups them into people you name. A People view shows each person's photos with their face
  outlined (as whole photos or face close-ups), lets you fix mistakes by dragging, offers possible
  matches to confirm, and shows the least alike faces first to find wrong ones. **Regroup** moves
  unnamed faces that clearly match someone you named. Finding faces can be paused. Names go
  into the photos' `.dvlp` files; face fingerprints stay on your computer.
- **Shared library:** **Share this library…** lets several computers (PCs and Macs) use one
  library's people, names, keywords and search; changes show on the others within seconds, and
  nothing one computer does overwrites another's.
- **Move to…:** move a folder to another drive or a file server safely: every file is copied and
  checked before the original goes, a stopped move carries on where it left off, and the
  library follows the folder.
- **Filmstrip** with ratings, sorting and filtering, a photo's saved versions grouped with it,
  **histogram and scopes**, **before/after split** (against the original or any saved version),
  and clipping warnings.
- **Copy and paste settings:** Ctrl+C / Ctrl+V (Cmd on a Mac), or right-click a photo.
  Paste onto the open photo or onto every selected photo in the filmstrip.
- **Network browsing:** the folder tree has a **Network** entry listing the computers and
  NAS boxes on your network and the folders they share; add one by address
  (`\\computer\share` on Windows, `smb://computer/share` on macOS). Windows asks for a
  password when a computer needs one; on macOS, shares are mounted through Finder and
  your Keychain. Big network folders open at once.
- **Non-destructive:** edits are saved in a small `.dvlp` sidecar file next to each photo;
  the original is never changed. Changes made by another program (an AI assistant, or a
  second copy of Develop) show at once in the open folder, and Undo goes back.
- **Export** to JPEG, PNG or TIFF, with resizing and metadata options, one photo or a
  batch (right-click a photo, or select several), with a progress bar.
- **Convert to DNG:** choose **DNG (original raw)** as the format in the Export dialog, or
  right-click raw photos and choose **Convert to DNG…**, to save the
  original raw data in Adobe's open DNG format, losslessly compressed (optionally with the
  original raw file embedded inside it). Develop writes the DNG itself, with the camera and
  lens details, the camera's baseline exposure (including Fujifilm DR200 / DR400
  compensation), and, for cameras whose distortion Lightroom always corrects (Nikon Z,
  Panasonic, Olympus / OM System, Fujifilm), that distortion and colour-fringe correction as a
  standard DNG lens-correction instruction, so other DNG readers correct it too. The correction
  comes from the Lensfun lens profile rather than the camera maker's own data, so it is close
  to, not identical with, Adobe's. Develop's edits come along in a `.dvlp` file beside each
  DNG, so Develop reopens it as you left it; other applications open the DNG unedited.

## AI assistants (MCP)

Develop includes an [MCP](https://modelcontextprotocol.io) server, so any AI assistant that
speaks MCP (Claude, Gemini, ChatGPT-based tools, local models in LM Studio, Cursor, VS Code
Copilot, Codex, Continue and others) can help with your photos: list a folder, read and change
edits, apply presets, rate, save and restore versions, look at a rendered preview or measure it with numbers (for AIs that cannot see images), export,
convert to DNG, search your library (by keyword, person or description) and add keywords. Develop also does the arithmetic-heavy jobs itself, so even small local
models handle a whole shoot quickly: **rate a folder by focus** (the sharpest frame of each
burst), **measure focus** at full size, and **set white balance automatically** (from skin or
neutral areas). Renders for the assistant happen without opening a window. It works with the photos' `.dvlp` files exactly as the app does and never
changes the original raw files. Each request to an AI is saved as one clearly labelled version
per photo (`AI · ...`), your edit is kept first (`Before AI · ...`), and versions you made can
never be changed or removed through MCP.

- `develop --mcp` for AI apps on your computer (they start it themselves).
- `develop --mcp-http <port>` for AI services that connect over the network: listens on this
  computer only and requires an access token.

**[docs/MCP.md](docs/MCP.md)** has the setup for each AI app, every tool and setting, good
practice, privacy notes and troubleshooting. **[docs/develop-skill](docs/develop-skill/SKILL.md)**
is a ready-made guide to give your assistant (as a Claude skill, custom instructions, a system
prompt or an `AGENTS.md`) so it edits carefully: saving versions first, previewing before
batch changes, and asking before exporting.

## Cameras

**Calibrated against Lightroom:** Sony, Nikon, Canon, Panasonic and Fujifilm. Develop's
sliders, white balance and Lightroom's per-camera adjustments (built-in lens corrections,
exposure offsets) were measured on reference photos from these brands' cameras.

**Supported, not yet calibrated:** every other camera the decoder reads, including
Olympus / OM System, Pentax, Leica, Samsung, Hasselblad, Phase One, Sigma, Kodak, Minolta,
Epson and DNG files from phones and other software. These open and edit normally, with
colour from the camera's own colour data (and Adobe's camera profile when Lightroom or
Camera Raw is installed), but they have not been compared with Lightroom yet: the same
slider value may look a little different, and the starting exposure may differ slightly.

To help calibrate a camera, export reference JPEGs from Lightroom Classic with the plugin
in `lightroom/` and share them.

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

### If Develop stops responding

When the window stops responding for more than a few seconds, Develop notes what it was doing
in `freeze.log` in its settings folder (`%APPDATA%\Develop` on Windows,
`~/Library/Application Support/Develop` on a Mac). Please attach that file to a bug report,
with what you were doing at the time.

## Building from source

Requires [Rust](https://rustup.rs/) (stable).

```bash
cargo build --release
```

The program is `target/release/develop` (`develop.exe` on Windows).

**AI noise reduction** uses the two RawNIND model files in `models/`, which come with the
repository, so a fresh clone has AI denoise for every supported format. **Face recognition**
(SFace) and **image understanding** (CLIP) need three larger model files that are not in the
repository: download them into `models/` as described in [models/README.md](models/README.md),
which also says where every model comes from and its license. Without them, Develop works but
has no People, suggestions or search by description.

**Installers** are built with [cargo-packager](https://github.com/crabnebula-dev/cargo-packager):

```bash
cargo install cargo-packager --locked
```

```bash
cargo packager --release
```

This makes an NSIS installer on Windows and a `.dmg` on macOS, in `dist/`. Pushing a tag such as `v0.3.3` builds both on GitHub Actions and attaches them to a release.
Releases follow [RELEASING.md](RELEASING.md); every release must update all the documentation
first, and the release workflow checks it.

## Lightroom reference plugin

`lightroom/DevelopReferences.lrplugin` is a Lightroom Classic plugin used to tune
Develop's sliders. For each selected photo it resets the photo, exports a blank
reference JPEG, then one JPEG per slider value (for example `DSC05615-cla+050.jpg`),
using a Lightroom export preset. It also covers the tone curve (the four regions and a set
of named point curves, such as `DSC05615-pc-mid+32.jpg`) and the Color Mixer (each
colour's hue, saturation and luminance, such as `DSC05615-hsl-s-blue-100.jpg`). Add it in
Lightroom with **File › Plug-in Manager › Add**, then run **Library › Plug-in Extras ›
Export Reference JPEGs…**.

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
- Face detector: YuNet (Shiqi Yu and others), MIT, from
  [opencv_zoo](https://github.com/opencv/opencv_zoo) (see `models/LICENSE-YuNet.txt`).
- Face recognition: SFace (Yaoyao Zhong and others), Apache-2.0, from
  [opencv_zoo](https://github.com/opencv/opencv_zoo) (see `models/LICENSE-SFace.txt`).
- Image understanding for keywords and search: CLIP ViT-B/32 (OpenAI), MIT, as converted by
  [Xenova/clip-vit-base-patch32](https://huggingface.co/Xenova/clip-vit-base-patch32) (see
  `models/LICENSE-CLIP.txt`).
- User interface: [egui / eframe](https://github.com/emilk/egui); GPU:
  [wgpu](https://github.com/gfx-rs/wgpu); ONNX inference:
  [ort](https://github.com/pykeio/ort).
- Adobe camera profiles are not included; Develop reads them only if Lightroom or Camera
  Raw is installed.

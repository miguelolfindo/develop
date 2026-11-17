## Develop 0.3.3

### Presets
- Save the current look as a **preset** (optionally with its white balance) and apply it to the
  open photo or to every selected photo with one click. Presets carry tone, presence, colour,
  curve, mixer, detail and profile settings; crop, rotation, lens correction and retouching stay
  as they are on each photo.

### Versions
- Keep several **versions** of a photo's edit: save, switch (Undo goes back), update and delete
  them, and compare any version with the current edit in the A/B split.
- Versions appear in the **filmstrip** as smaller cards (V1, V2…) grouped with their photo, each
  with a thumbnail of that version. Click one to open the photo at that version.
- Versions are saved in the photo's `.dvlp` file, so they travel with it.

### Black & White
- A **Color / Black & White** treatment switch in the Profile section. In black and white, the
  Color Mixer becomes a **B&W Mix**: one slider per colour sets how bright it turns out in grey
  (darker skies, brighter skin and sand).

### AI assistants (MCP)
- Develop now includes an **MCP server**, so an AI assistant (Claude, Gemini, ChatGPT-based tools,
  local models in LM Studio, Cursor, VS Code Copilot, Codex and others) can help with your photos:
  list a folder, read and change edits, apply presets, rate, save and restore versions, export,
  and convert to DNG.
- Assistants can **look at a preview**, or **measure a photo with numbers** (histogram, clipping,
  colour cast, contrast, sharpness) so even models that cannot see images can check their work.
- **Your work is protected:** your edit is kept as a `Before AI` version before an assistant
  changes anything, each request's changes go into one clearly labelled `AI ·` version per photo,
  and versions you made can never be changed or removed by an assistant.
- Setup for each AI app and everything else is in the new [MCP guide](docs/MCP.md).

### Documentation
- A complete **[user guide](docs/USER-GUIDE.md)** covering every feature, keyboard shortcuts,
  where edits are kept and troubleshooting.

### Fixes
- **Ctrl+C / Ctrl+V** for copying settings stopped working when the clipboard held a picture
  (a screenshot, for example) or nothing at all. Copying now also puts a line of text on the
  clipboard, so pasting always works.

### Installing
- **Windows:** run `develop_0.3.3_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.3_aarch64.dmg`, see below.

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
- The Color Mixer and B&W Mix are calibrated on one reference photo so far; purple and magenta
  are approximate.
- Changes an AI assistant makes while Develop is open on the same photo show when the photo is
  opened again.
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.

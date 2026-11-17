## Develop 0.3.5

### Skin white balance
- A new **Skin** button next to **Auto** in the White Balance panel sets the white balance so the
  **people** look natural, whatever the rest of the picture shows: useful when a face in shade
  sits beside a sunlit white wall, or whenever the people matter more than the room.
- Temperature sets how strongly coloured the skin is (cooling it under orange light, never
  warming pale skin), and tint turns its hue to natural: a little green when skin looks pink, a
  little magenta when it looks green-yellow.
- It needs a face; without one, it changes nothing and says so. Exposure is never changed, and
  **Undo** goes back.

### Auto white balance fixes
- Faces too dark to judge (night scenes, backlight) are now brightened for measuring, instead of
  being treated as if there were no faces.
- Faces lit by strongly coloured light (blue or purple stage and LED light) are left alone, with a
  note saying the light is too coloured for white balance to make skin natural. Before, the note
  wrongly said there were no faces.
- Whether a photo has enough grey to go by is now judged at the photo's own brightness. A wall
  that only looked grey in the brightened measuring render could make Auto change a photo it
  should have left alone.

### AI assistants (MCP)
- `auto_white_balance` takes `from: "skin"` to do what the Skin button does.

### Installing
- **Windows:** run `develop_0.3.5_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.5_aarch64.dmg`, see below.

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
- Auto white balance's firelight setting was judged by eye on two firelit photos; firelit photos
  with nobody in them are left as the camera made them (still orange).
- **Auto** works on the open photo; to do many photos at once, use an AI assistant's
  `auto_white_balance`.
- Focus measurements are a guide, best for comparing frames of the same scene; very noisy
  photos read less reliably.
- The Color Mixer and B&W Mix are calibrated on one reference photo so far; purple and magenta
  are approximate.
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.

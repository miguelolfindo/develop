## Develop 0.3.4

### Auto white balance
- A new **Auto** button in the White Balance panel sets the white balance for you. When there
  are people in the photo it goes by their **skin**: it finds the faces, judges their skin as if
  they were correctly exposed, and cools (or warms) the picture until skin looks natural without
  turning pink. That is what makes **firelight, candles and tungsten** come out right, where the
  camera leaves everything orange and nothing in the picture is grey.
- Without faces it makes the grey and white parts of the picture neutral; with nothing to go
  by, it leaves the photo alone and says so.
- Only temperature (and tint, from grey areas) changes, never exposure. A note under the button
  says what it went by, and **Undo** goes back. Running it again gives the same answer.

### Edits from other programs show at once
- While a folder is open, Develop notices when another program changes a photo's edit (an AI
  assistant, or a second copy of Develop): new ratings and versions appear in the filmstrip, and
  the photo you have open updates, with a note saying so. **Undo** brings back what you had.
- The newest change wins, so neither program saves an old copy over the other's. Develop is told
  by the system when files change, so it does not keep checking the folder.

### AI assistants (MCP)
- **Rate a shoot by focus:** the new `cull_photos` tool measures every photo's focus at full
  size, groups bursts by when they were taken, and suggests 4 stars for the sharpest frame of
  each burst, 3 for frames nearly as sharp and 1 for the rest. A 400-photo folder takes minutes
  instead of hours, and measurements are remembered, so nothing is measured twice.
- **Focus checks:** `check_focus` measures one photo's focus at full size and shows the
  assistant its sharpest area at 100%; `check_photos` can add focus measurements to every row,
  and now says when each photo was taken.
- **Auto white balance for assistants:** `auto_white_balance` does what the Auto button does,
  for a list of photos, four at a time.
- **Better colour readings:** `check_photo` now reports the photo's own (as-shot) white balance,
  so an assistant can make changes relative to it, and a **skin** reading for photos with
  people.
- **No more windows:** checks and previews are rendered without opening a Develop window, so
  nothing flashes up on screen while an assistant works, and each is about a second faster.
- Long checks stop after about 35 seconds and say where to continue, so assistants with a
  60-second tool limit no longer time out.
- The guide for assistants (`docs/develop-skill`) has new workflows for culling and correcting
  colour, and a rule of thumb for white balance steps.

### Fixes
- An assistant's edits could be overwritten by Develop when it was open on the same photo, and
  the other way round. Both now read the latest edit before saving.

### Installing
- **Windows:** run `develop_0.3.4_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.3.4_aarch64.dmg`, see below.

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
- Auto white balance's skin target was set from one firelit photo judged by eye and checked on
  others; firelit photos with nobody in them are left as the camera made them (still orange).
- **Auto** works on the open photo; to do many photos at once, use an AI assistant's
  `auto_white_balance`.
- Focus measurements are a guide, best for comparing frames of the same scene; very noisy
  photos read less reliably.
- The Color Mixer and B&W Mix are calibrated on one reference photo so far; purple and magenta
  are approximate.
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.

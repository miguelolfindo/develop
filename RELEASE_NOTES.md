## Develop 0.4.0

Develop now helps you find and sort photos, not only edit them: keywords, a search bar, a
library of folders searched together, faces recognised and grouped into people you name, and
keyword suggestions from what is in each photo. Everything runs on your computer.

### A new layout
- A bar above the panels: **Folders** and **Faces & Metadata** on the left choose what the left
  panel shows, **Inspector** on the right shows or hides the editing panel. The one showing is
  highlighted; click it again to hide its panel.
- Drag the edges of the left and right panels to make them wider or narrower, and the top edge
  of the filmstrip to make it taller: its thumbnails grow with it. Develop remembers the sizes.
- **Grid view:** the **Grid** button at the top of the filmstrip shows the thumbnails across the
  whole window, in rows that run edge to edge, with a **Size** slider at the bottom right.
  Double-click a photo to go back to it with the filmstrip underneath.
- The Inspector shows the file name on a line of its own, **when the photo was taken** under it,
  and Undo, Redo and Reset All below the camera line. A thick dark line marks where the scrolling
  editing sections begin.

### Keywords and search
- **Keywords** (Faces & Metadata): type a word and press Enter (commas for several); click × to
  remove one. With several photos selected, keywords go onto all of them. They are kept in each
  photo's `.dvlp` file, so they travel with it.
- **Suggested keywords:** Develop looks at the photo and offers words for what is in it (dog,
  beach, restaurant, car…); click one to add it. Nothing is added unless you click.
- **Search bar** at the top of the filmstrip: keywords, people's names and file names. Switch it
  from **This folder** to **Library** to search every library folder at once, and to also find
  photos by describing them ("people eating noodles", "surfboard on the sand").

### The library
- Add the folders you want searched together under **Library** (Faces & Metadata, **+ Add**), or
  right-click any folder under Folders and choose **Add to Library**.
- Develop reads each library photo once, in the background, to find faces and see what is in it.
  Progress shows above People and in the top bar ("Finding faces 37 of 410"). Closing Develop
  while it is still reading asks first; what is done is kept and the rest is read next time.

### People
- Faces found across the library are grouped into people, shown as a grid of faces under
  **People**, named people first. **Find a person** filters by name; long lists show their first
  rows until **Show all**.
- Click a face to open the **People view**: a large close-up of the face, a box to type their
  name (Enter saves), and every photo they are in with the face outlined. Hover the magnifying
  glass on a photo for a close-up of the face in it.
- Fix mistakes: × takes a photo out of the set; click, Ctrl-click or Shift-click to pick several
  and drag them onto the right face in the People list. Naming two groups the same joins them.
- **Possible matches:** below a line, faces that look somewhat like this person but were not
  sure enough to join them. Click the tick on the right ones, or pick several and drag them up.
- Names are written into the photos' `.dvlp` files, so you can search for them. The face
  fingerprints themselves stay in Develop's settings folder, never in your photos.

### AI assistants (MCP)
- New tools: `search_photos` (by keyword, person, file name or description, across the library or
  one folder), `set_keywords`, `suggest_keywords` and `list_people`.
- `list_photos` and `get_settings` now include each photo's keywords and people.

### Fixes
- Typing a person's name and pressing Enter now saves it.
- Scrolling the filmstrip or grid to the open photo no longer scrolls the editing panel too.

### Installing
- **Windows:** run `develop_0.4.0_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.4.0_aarch64.dmg`, see below.

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
- Face recognition and keyword suggestions read each photo's embedded preview; very small or
  blurred faces may be missed, and possible matches sometimes include people who only look alike.
- Searching by description and keyword suggestions work in English.
- Searching by description covers library folders only; **This folder** searches keywords,
  people and file names.
- The installer is larger (about 190 MB more) because it now includes the face recognition and
  image understanding models.
- Auto white balance's firelight setting was judged by eye on two firelit photos; firelit photos
  with nobody in them are left as the camera made them (still orange).
- Focus measurements are a guide, best for comparing frames of the same scene; very noisy
  photos read less reliably.
- The Color Mixer and B&W Mix are calibrated on one reference photo so far; purple and magenta
  are approximate.
- DNG lens corrections come from Lensfun lens profiles, so they are close to, not identical
  with, Adobe's built-in corrections.
- Cameras outside the five calibrated brands may differ slightly from Lightroom.

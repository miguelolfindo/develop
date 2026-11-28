## Develop 0.5.3

A small update with two export fixes.

### Fixed
- **Show in folder** after an export opened your Documents folder instead of the one you exported
  to, whenever the folder's name or path had a space in it. It now opens the right folder, with
  the exported photo selected. Show in File Explorer on photos and faces is fixed the same way.
- The **Export…** button in the Export dialog was easy to miss. It is now the large orange
  button at the bottom, like the one in the Inspector.

### Installing
- **Windows:** run `develop_0.5.3_x64-setup.exe`, or update from inside Develop. It installs
  over an earlier version and keeps your settings, library and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** `Develop_0.5.3_aarch64.dmg`, see below.
- Coming from 0.4.1 or earlier: the first time it opens, it moves the library into its new form.
  The old files are left as they were, so an older Develop still works.
- Coming from 0.5.1 or earlier: see what came in 0.5.2 below (masks, perspective correction,
  naming people on the photo), including its note about older versions.

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

---

## What came in Develop 0.5.2

Masks: sliders that change only the parts of a photo you paint, perspective correction, and
naming people right on the photo.

### Masks
- **Masks** (Inspector, above Retouch): click **Brush** (or press **K**) and paint on the photo
  where the sliders should apply. Each mask has its own **Temp**, **Tint**, **Exposure**,
  **Contrast**, **Highlights**, **Shadows**, **Whites**, **Blacks**, **Texture**, **Clarity**,
  **Dehaze** and **Saturation**, added to the photo's own sliders where it is painted.
- Up to 8 masks per photo: **+ New** starts another one. Tick or untick a mask to compare, rename
  it, or delete it with **×**.
- **Add** and **Erase** brushes (hold **Alt** for the other one), with **Size**, **Feather** and
  **Flow**; `[` `]` change the size and Shift+`[` `]` the feather, as in Retouch.
- **Auto Mask** keeps the brush to areas that look like the one under its centre, so it stops at
  edges: paint a sky with the centre in the sky, and roofs, trees and people are left out.
- The mask being painted shows in red (**O** shows or hides it).
- Masks are saved with the photo's edit, in versions and in Undo, and copied with Copy settings.
  Presets leave them alone. AI assistants see which masks a photo has.

### Perspective correction (Transform)
- A new **Transform** section straightens converging lines: buildings leaning in, a wall seen at
  an angle, a tilted horizon.
- **Upright** finds the photo's long straight lines and sets the correction: **Level** (turn
  only), **Vertical** (upright lines stand straight), **Full** (level lines too) and **Auto** (a
  balanced Full).
- **Guided:** drag along two to four things that should be upright or level, and the photo turns
  to match as you draw.
- **Vertical**, **Horizontal**, **Rotate**, **Aspect**, **Scale** and **X / Y Offset** sliders, as
  in Lightroom. The correction follows the lens's focal length from the photo, and the crop stays
  inside the corrected photo. **Grid** shows lines over the photo while you judge it.
- Saved with the photo's edit and in versions; AI assistants can set the sliders too.

### Name people on the photo
- **Faces** at the bottom of the photo (or **F**) outlines the faces found in it, with each
  person's name under their face. Click a face to name it; names you already use come up as you
  type.
- Naming a face nobody has named yet names the faces grouped with it too, as under People. A face
  that already has a name is corrected on its own, and **Not (name)** takes the name off it.
- Works on photos outside your library too: their faces are found when you first show them.

### Known limitations
- The mask sliders use the same maths as the main sliders, but have not yet been compared with
  Lightroom's masks on reference exports, so the same numbers may look a little different.
- A mask's edge is worked out on a copy of the photo about 2,400 pixels across, so at 100% zoom
  on a big photo its edge is soft rather than pixel-sharp.
- No gradient or radial masks, and no AI subject or sky selection yet.
- Upright and the Transform sliders have not yet been compared with Lightroom's on reference
  photos, so the same numbers may correct a little more or less. Guided lines are not kept with
  the photo (the correction they set is).
- Upright looks at a copy of the photo about 1,200 pixels across: photos with few long straight
  edges (night scenes, nature) may give little or no correction; Guided works on any photo.
- An older Develop shows these photos without their masks and perspective correction, and
  editing one there removes them. Update every computer that edits the same photos (a shared
  library included).

---

## What came in Develop 0.5.1

A small update that helps find the cause when Develop stops responding.

### New
- If the window stops responding for more than a few seconds, Develop now writes down what it
  was doing in `freeze.log` in its settings folder, and notes when it responds again. If Develop
  ever freezes for you, please send that file along with what you were doing.

---

## What came in Develop 0.5.0

A big library on a network drive now stays fast, can be shared between computers, and can be
moved safely. Plus noise reduction for every photo, quick ways to tidy up people, and much
smaller updates.

### Share a library between computers
- Right-click a library folder (Faces & Metadata, Library) and choose **Share this library…**.
  Any other computer that adds the same folder to its library, another PC or a Mac, sees the same
  people, names, keywords and search. Changes made on one show on the other within seconds.
- Each new photo is read on one computer only, whichever gets to it first.
- If two computers change the same thing, nothing is lost: the newer name wins, and the People
  view says which computer gave the name ("named on MacBook-Air").
- **Stop sharing…** stops on this computer only (you can come back with **Use the shared library
  again**) or for everyone. Either way, every computer keeps a full copy, and the shared data is
  kept as a backup.

### Move a folder safely
- Right-click a folder and choose **Move to…**. A summary comes first: how many photos and files,
  their size, the free space at the destination, and any names already taken there.
- Every file is copied, read back and checked before the original is removed. If the move stops
  part-way, run it again and it carries on from where it stopped.
- On the same drive or network share, files are simply renamed, which is instant.
- **Keep both** gives a moved file a number (DSC01234-2.ARW) when its name is taken by a different
  photo, and its edits go with it. Nothing at the destination is ever replaced.
- The library follows the folder, so faces, names and search stay as they were.
- The progress shows "Checking what is already there" first, and the numbers sit above the bar
  where you can read them.

### Noise reduction for every photo
- **Detail › Noise Reduction**, as in Lightroom: **Luminance** with **Detail** and **Contrast**,
  and **Color** with **Detail** and **Smoothness**. It updates live as you drag, and works on
  JPEG, PNG, TIFF and HEIC as well as raw files.
- It removes single bright and dark specks and smooths grain and colour blotches without plastic
  patches. A trace of fine grain is kept, so smooth areas still look natural.
- AI Denoise stays for raw files. The new sliders are saved in presets and versions, and AI
  assistants can set them too.
- Sliders now sit on one row (name, track, value), as in Lightroom, so more of them fit.

### People
- **Regroup** (above the People grid) moves unnamed faces that clearly match someone you have
  named into their set. Borderline faces wait for you under Possible matches.
- Faces remember who they are **not**. A face you take out of someone's set is never put back by
  Regroup or offered as a possible match.
- While you type a name, names you already use come up: press Tab or click to use one.
- **Show** above a person's photos: **All**, **Least alike first**, or **Least like** the person.
  The last two bring wrong faces to the top: other people, things that aren't faces, and tiny
  faces. Nothing is taken out unless you take it out.
- **Cards: Photo / Face** shows each card as the whole photo or as the face up close.
- **Pause** stops finding faces for now and stays paused, also after closing Develop, until you
  press **Resume**. It's in the status at the top and in the Library section. Finding faces also
  pauses while a folder is being moved.

### Much faster with big libraries
- Develop shows a start-up box at once, with a progress bar, while it reads the library. On a
  library of 60,000 faces, the window is ready in about 5 seconds instead of more than 30.
- The window stays smooth while faces are being found. Naming a big group no longer freezes it,
  and neither does **Show all** in People or a person with thousands of photos.
- Finding faces is about five times faster per photo.
- Network folders open at once, even with thousands of photos. Capture dates are read in the
  background and remembered. **Network** lists your file server within a few seconds.
- The library is kept in a smaller, faster form. A drive that is asleep or unplugged no longer
  makes the library forget its photos.

### Smaller updates
- When you update from inside Develop, it downloads a small package of about 26 MB if your AI
  models are already up to date, instead of the full 228 MB installer. Otherwise it downloads the
  full installer, as before.

### Also new
- Right-click any photo and choose **Show in File Explorer** (Windows) or **Show in Finder** (Mac).

### Known limitations
- A shared library needs every computer on Develop 0.5.0.
- See also the limitations listed for 0.4.1 and 0.4.0 below.

---

## What came in Develop 0.4.1

### EXIF in Faces & Metadata
- A new **EXIF** section at the top of Faces & Metadata shows how the photo was taken, as the
  camera recorded it: when, camera, lens, focal length, shutter speed, aperture, ISO, exposure
  compensation, size, file size, artist, copyright and location. **Copy location** copies the
  coordinates, to paste into a map.
- It shows the open photo, or the photo picked in the filmstrip or the People view. The photo's
  name heads the panel, above EXIF and Keywords.

### Sections remember being open or closed
- EXIF, Keywords, People and Library in Faces & Metadata now collapse and expand by clicking
  their titles, like the Inspector's sections.
- Every section, in both panels, remembers whether you left it open or closed. On first launch
  they all start open (Presets, Versions, Crop, Lens Correction and Retouch used to start closed).

### Fixes
- Updating on Windows: Develop now waits until it has fully closed before starting the installer,
  so the installer no longer says Develop is still running. It may still ask if another copy is
  open, such as a second window or an AI app's Develop connection; clicking OK closes it.
- Windows now shows Develop's real version in the file's details (it showed 0.3.1).

### Known limitations (0.4.1)
- JPEGs show the camera details their EXIF holds; shutter speed and ISO show for camera raw files.

---

## What came in Develop 0.4.0

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

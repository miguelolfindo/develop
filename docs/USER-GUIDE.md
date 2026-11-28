# Develop user guide

Develop is a fast, free raw photo editor for Windows and macOS. Every adjustment runs on the
graphics card, so sliders respond instantly even on 61-megapixel files, and the sliders are
tuned against Lightroom Classic so a given value looks much as it does there. This guide
covers everything the app does, in the order you would use it.

**Contents**

1. [Installing and updating](#1-installing-and-updating)
2. [The window](#2-the-window)
3. [Finding and organising photos](#3-finding-and-organising-photos)
   - [Grid view](#grid-view) · [Search](#search) · [EXIF](#exif) · [Keywords](#keywords)
     · [The library](#the-library) · [Sharing a library](#sharing-a-library) · [People](#people)
     · [Moving a folder](#moving-a-folder)
4. [Looking at a photo](#4-looking-at-a-photo)
5. [Editing](#5-editing)
   - [Presets](#presets) · [Versions](#versions) · [Profile and treatment](#profile-and-treatment)
     · [Crop and straighten](#crop-and-straighten) · [White balance](#white-balance)
     · [Tone](#tone) · [Presence](#presence) · [Tone curve](#tone-curve)
     · [Color mixer](#color-mixer) · [Black and white mix](#black-and-white-mix)
     · [Detail](#detail) · [Lens correction](#lens-correction) · [Transform](#transform)
     · [Masks](#masks)
     · [Retouch](#retouch)
6. [Undo, reset, copy and paste](#6-undo-reset-copy-and-paste)
7. [Exporting](#7-exporting)
8. [Converting to DNG](#8-converting-to-dng)
9. [Where your edits are kept](#9-where-your-edits-are-kept)
10. [AI assistants](#10-ai-assistants)
11. [Keyboard and mouse](#11-keyboard-and-mouse)
12. [Supported files and cameras](#12-supported-files-and-cameras)
13. [Command line](#13-command-line)
14. [Troubleshooting](#14-troubleshooting)
15. [Privacy](#15-privacy)

---

## 1. Installing and updating

### Windows

Download `develop_x.y.z_x64-setup.exe` from the [Releases](https://github.com/miguelolfindo/develop/releases) page and run it.
It installs for your user account only (into `%LOCALAPPDATA%\Develop`), so no administrator
rights are needed. Installing a newer version over an older one keeps your settings and edits.

### macOS

Apple silicon Macs with macOS 13.4 or later. Download `Develop_x.y.z_aarch64.dmg`, open it, and
drag Develop into Applications. Develop is not signed with an Apple developer certificate, so
macOS blocks the first launch: open **System Settings › Privacy & Security**, scroll to
**Security** and click **Open Anyway**, then open Develop again. The README's
[Installing on macOS](../README.md#installing-on-macos) section covers the "damaged" message and
other first-launch problems.

### Updates

When Develop starts it checks GitHub for a newer release (in the background, never blocking
you). If there is one, a dialog shows what is new, with three choices:

- **Download and install**: downloads the installer with a progress bar, then closes Develop
  and installs the update (on a Mac it replaces the app and opens the new version).
- **Later**: asks again next time.
- **Skip this version**: does not offer this version again (a later one will be offered).

An update is only offered once its installer for your system is ready to download.

When the AI models on your computer are already the ones the new version uses, Develop downloads
a small update package (about 26 MB) instead of the full installer (about 228 MB), and installs it
once Develop has closed. Otherwise it downloads the full installer, as before.

**Starting up:** while Develop reads your library, a box shows its name, version and a progress
bar saying what it is reading. The full window follows a few seconds later.

### Uninstalling

Windows: **Settings › Apps**, or run `uninstall.exe` in `%LOCALAPPDATA%\Develop`. macOS: drag
Develop from Applications to the Bin. Your photos and their `.dvlp` edit files stay where they
are; the settings folder (see [section 9](#9-where-your-edits-are-kept)) can be deleted by hand.

---

## 2. The window

```
┌──────────────────────────────── top bar ────────────────────────────────┐
│ Folders | Faces & Metadata                                  Inspector   │
├──────────────┬────────────────────────────────────────┬─────────────────┤
│  left panel  │            photo (viewer)              │   inspector     │
│              │  zoom · A/B Split · Clipping           │  [ Export… ]    │
├──────────────┴──────────────── filmstrip ─────────────┴─────────────────┤
```

- **Top bar:** the open photo's file name, camera, size and bit depth; while Develop is reading
  new library photos, "Finding faces 37 of 410" with a **Pause** button; and which graphics system Develop is using
  (Vulkan, DX12, Metal, OpenGL or WebGPU).
- **Tabs:** **Folders** and **Faces & Metadata** choose what the left panel shows; **Inspector**
  shows or hides the right panel. The tab showing is highlighted and joins its panel; click it
  again to hide the panel (`[` and `]` do the same).
- **Left panel:** **Folders** shows your drives, home folder and network. **Faces & Metadata**
  shows the photo's name, then **EXIF**, **Keywords**, **People** and **Library**.
- **Viewer** (middle): the photo, with the zoom readout, **A/B Split** and **Clipping** in the
  bottom-left corner. It also shows the [grid view](#grid-view) and the
  [People view](#the-people-view).
- **Inspector** (right): the photo's file name and type, when it was taken, the camera, size and
  as-shot white balance (hover for details), then **Undo**, **Redo** and **Reset All**, and the
  View, Histogram and Vectorscope sections. A thick dark line marks where the editing sections
  begin; they scroll below it. **Export…** sits at the bottom.
- **Filmstrip** (bottom): the photos of the current folder, or of a search.

**Sizes:** drag the edge of the left or right panel to make it wider or narrower, and the top edge
of the filmstrip to make it taller (its thumbnails grow with it). Develop remembers the sizes, the
tab and the grid view for next time.

Sections, in the Inspector and in Faces & Metadata, collapse and expand by clicking their titles.
Develop remembers which are open for next time; the first time, they are all open.
Double-clicking a slider's name puts it back to its default.

---

## 3. Finding and organising photos

### Opening photos

- Click a folder in the folder panel to show its photos in the filmstrip, then click a photo.
- **Ctrl+O** (Cmd+O on a Mac) opens a file with the system's file dialog.
- Drag a photo file onto the window.
- When Develop starts it comes back to the folder and photo you had open last time.

### Folder panel

Lists your drives, your home folder (and **Volumes** on a Mac) and **Network**. Click the arrow
next to a folder to open it, click its name to show its photos. **Read again** (the circular
arrow) reads the drives, network and folders again.

Right-click a folder and choose **Add to Library** to have it searched with the others and its
faces recognised (see [The library](#the-library)); a folder already in the library offers
**Remove from Library**. **Move to…** moves the folder somewhere else safely (see
[Moving a folder](#moving-a-folder)).

Big folders on a network drive open at once: the photos show first in the order of their file
times, and settle into capture-date order a moment later, as the dates are read in the
background. Develop remembers the dates, so the folder opens in order next time.

**Network** lists the computers and NAS boxes on your network and the folders they share. To
add one that is not listed, type its address in the box at the top of Network (`\\computer\share`
on Windows, `smb://computer/share` on a Mac) and confirm. Windows asks for a user name and
password if the computer needs them; on a Mac, use **Connect…**, which signs in through Finder
and your Keychain. Right-click an added computer and choose **Remove from Network** to remove it.
Computers you added, and those found last time, show at once; on Windows the others are added as
they are found ("Still looking for others…").

### Filmstrip

Each card shows the photo, its file type (amber for camera raw files, teal for DNG and pictures),
its star rating, a sliders mark when it has been edited, and a stacked-frames mark with a number
when it has saved versions.

- **Click** a card to open that photo. **Ctrl-click** (Cmd-click) adds or removes photos from the
  selection; **Shift-click** selects everything from the open photo to the one clicked.
- **Left and right arrow keys** move to the previous or next photo.
- **Sort** by Capture Date, File Name or File Type, ascending or descending (the arrow button).
- **Filter** to show only rated photos, a minimum number of stars, or five stars only.
- **Refresh** (the circular arrow) reads the folder again for new or deleted files.
- **Search** the folder or the whole library (see [Search](#search)).
- **Grid** shows the thumbnails across the whole window (see [Grid view](#grid-view)).
- The header shows the folder, which photo of how many is open, and how many are selected.
- The mouse wheel scrolls the strip sideways.
- Drag the top edge of the filmstrip to make it taller or shorter; the thumbnails follow.

**Versions in the filmstrip.** A photo with saved [versions](#versions) is shown together with
them in one tinted frame: the photo's own card first, then a smaller card per version, marked
**V1**, **V2**… with the version's name underneath. Each version card shows a thumbnail of that
version (drawn once the photo has been opened; until then the photo's own thumbnail, dimmed).
Click a version card to open the photo at that version; right-click it to **Open this version**
or **Delete this version**.

### Grid view

Click **Grid** at the top left of the filmstrip to show the thumbnails across the whole main
window. The rows run from edge to edge: the **Size** slider at the bottom right sets about how big
the thumbnails are, and each row stretches them a little to fill the width (a last row that is not
full keeps the same size). Click a photo to open it (the Inspector follows it); double-click it,
or click **Grid** again, to go back to the photo with the filmstrip underneath. Selecting, rating,
sorting, filtering and searching work as in the filmstrip. A photo's saved versions show as the
number in its corner; their own cards appear in the filmstrip.

### Search

Type in the box at the top of the filmstrip. Every word must appear in a photo's keywords, the
names of the people in it, or its file name.

- **This folder** searches the photos of the open folder.
- Click it to switch to **Library**: every [library](#the-library) folder at once. Photos whose
  words match come first; after them come photos whose **content** fits what you typed, so you can
  describe what you are looking for: "people eating noodles", "surfboard on the sand", "red car at
  night". The header says how many matched each way, for example *Library: "dinner" (4 by keyword,
  20 by content)*.
- **×** clears the search.

Searching by content works in English, on library photos Develop has already read.

### EXIF

At the top of **Faces & Metadata**, under the photo's name, **EXIF** shows how the photo was taken,
as the camera recorded it: when it was taken, the camera, lens, focal length, shutter speed,
aperture and ISO, exposure compensation, the size in pixels (for the open photo), the file size,
and the artist, copyright and location when the camera recorded them. **Copy location** copies
the coordinates, to paste into a map. Hover a long value to see it whole.

It shows the open photo, or the photo selected in the filmstrip or picked in the People view; with
several picked, pick one to see its details. JPEGs show what their EXIF holds, which may not
include shutter speed and ISO.

### Keywords

Keywords are words to find photos by. They are in **Faces & Metadata**, under **KEYWORDS**, for
the open photo (or the selected photos; the name at the top of the panel says which).

- Type a word and press **Enter** to add it; separate several with commas.
- Click **×** on a keyword to remove it.
- With several photos selected, words you add go onto all of them, and each keyword shows how
  many have it, for example *dinner (2 of 3)*.
- **Suggested (click to add):** Develop looks at the photo and offers words for what is in it
  (dog, beach, restaurant, car, flower…). Click one to add it; nothing is added unless you click.
  The suggestions come from a built-in list of about 80 common photo subjects; to use your own
  words, put them in `keyword-vocabulary.txt` in the [settings folder](#9-where-your-edits-are-kept),
  one per line.
- **People:** the named people in the photo. **×** takes someone off it (a wrong match).

Keywords are kept in the photo's `.dvlp` file, so they travel with it.

### The library

The library is the set of folders you want searched together and whose faces Develop recognises.
It is under **LIBRARY** in Faces & Metadata.

- **+ Add** picks a folder (its subfolders are included). Or right-click a folder under Folders and
  choose **Add to Library**.
- Click a library folder to show its photos; right-click it to **Remove from Library**.
- The circular arrow reads the library again.

Develop reads each library photo once, in the background: it finds the faces and looks at what is
in the photo, from the photo's embedded preview (about two photos a second). While it does,
**Finding faces and reading content** shows a progress bar and the time left above People, and the
top bar shows "Finding faces 37 of 410" whatever tab is open. It checks the library again when it
starts and every ten minutes, and only new or changed photos are read again. A photo moved to
another folder keeps its faces and names. A folder Develop cannot reach (a network drive that is
asleep, a disk that is unplugged) keeps its photos in the library; photos that are gone are kept
for 30 days in case they come back.

**Pause** (next to the progress, and in the top bar) stops finding faces for now: what is done is
kept, and it stays paused, also after closing Develop, until you click **Resume**. On a big
library finding faces can take many hours, so pause it when you want the computer for something
else. It also pauses by itself while a folder is being moved.

**Closing while it reads:** Develop asks first. **Keep running** carries on; **Quit now** keeps
everything read so far (it is also saved every 20 seconds as it goes), and the rest is read the
next time Develop opens.

### Sharing a library

One library can be used on several computers, for example a PC and a Mac sharing photos on a
file server. Right-click a library folder under **LIBRARY** and choose **Share this library…**.

- Develop keeps the library's face and search data in a hidden `.develop` folder inside it. Your
  photos and their `.dvlp` files don't change.
- On the other computer, add the same folder to its library (on a Mac, the share under
  `/Volumes`). Its people, names, keywords and search are there straight away, without reading
  the photos again.
- Changes made on one computer show on the other within seconds. New photos are read by one
  computer only, whichever gets there first.
- Nothing is overwritten: when both computers change things at once, both changes are kept. If
  two computers rename the same person at about the same time, the later name wins; a name given
  on another computer shows which computer gave it, for example "named on MacBook-Air".
- Each computer also keeps its own copy of the names, for when the folder can't be reached.
- **Stop sharing…** offers **Stop on this computer** (the others carry on; come back later with
  **Use the shared library again**) or **Stop for everyone** (the `.develop` folder is renamed
  `.develop-stopped-<date>` and kept as a backup). Either way, this computer keeps a full copy and
  carries on as a library of its own.

Every computer sharing a library needs Develop 0.5.0 or later.

### Moving a folder

Right-click a folder under Folders and choose **Move to…**, then pick where it should go. A
summary shows the photos and files, their size, the free space there, and any files with the
same names already at the destination. Click **Move** to go ahead.

- On the same drive or network share, files are renamed, which is instant.
- To another drive, every file is copied, read back and checked before anything is removed. The
  originals are removed only once all of them are safely copied. If the move stops part-way,
  nothing is lost: run it again and it carries on, skipping what is already there.
- The progress shows "Checking what is already there" first, then the copying. **Cancel** stops
  it at any time.
- **Keep both:** if files with the same names but different photos are already there, tick it to
  give the moved files a number (`DSC01234-2.ARW`), with their `.dvlp`, JPEG and `.xmp`. Nothing
  at the destination is ever replaced.
- Edits are saved first. The library follows the folder: faces, names and search stay as they
  are, and if the folder moves out of the library its new place is added.

### People

Faces found in library photos are grouped into people. Under **PEOPLE** in Faces & Metadata:

- A grid of faces: named people first, then unnamed groups, the largest first, each with how many
  photos it is in. Groups of a single photo are left out.
- **Find a person** filters by name. A long list shows its first three rows until **Show all**.
- Click a face to open it in the People view. Right-click it to **Show their photos in the
  filmstrip**.
- **Regroup** checks the faces not named yet against the people you have named, and moves the
  clear matches to them. Borderline faces stay for you to confirm under Possible matches. A note
  says how many moved, and names anyone skipped because their own photos are too mixed to go by.

#### The People view

Clicking a face fills the main window with that person:

- A large close-up of their face, and **Name**: type their name and press **Enter**. Names you
  already use that fit what you typed come up underneath: **Tab** or click to use one. Naming two
  groups the same joins them into one person.
- **Could this be one of them?** lists named people who look alike; click a name to put these
  photos with theirs.
- Every photo they are in, with the face outlined in amber. Hover the **magnifying glass** in a
  photo's corner for a close-up of the face in it, useful when a face is small.
- **Cards: Photo / Face:** show each card as the whole photo, or as the face up close (easier to
  tell who it is).
- **Show:** **All** the photos, **Least alike first**, or **Least like** the person (only the faces
  well below the rest in likeness). The last two bring wrong faces to the top: other people,
  things that aren't faces, tiny faces, or the person as a small child. Nothing is taken out
  unless you take it out.
- **×** in a photo's corner (on hover) takes it out of the set. Develop remembers that the face is
  not this person: Regroup and Possible matches never put it back.
- **Picking several:** click a photo to pick it, **Ctrl-click** (Cmd-click) to add or remove one,
  **Shift-click** to pick a run. With several picked, **Take N out of this set** (or the Delete key)
  removes them. While photos are picked, the Keywords section works on them.
- **Moving photos to someone else:** drag a photo (or the picked photos) onto the right face under
  People. A small copy follows the pointer and says where it will go ("Move to Talia"); the face
  under the pointer is ringed in teal.
- **Possible matches:** below a thin line, faces that look somewhat like this person but were not
  sure enough to join them, most alike first, outlined in teal. Click the **tick** on the right
  ones, or pick several and drag them up above the line (or use **Add N possible matches**).
  Some may be people who only look alike: you decide.
- Double-click a photo to open it; the filmstrip then shows this person's photos.
- **Back to the photo** (or Esc) returns to the photo.

People's names are written into their photos' `.dvlp` files (like keywords), so a search finds
them and they travel with the photos. The face fingerprints that recognise people stay in
Develop's settings folder and are never written into your photos or sent anywhere.

### Ratings

Press **1** to **5** to give the selected photos (or the open photo) that many stars, **0** to
remove the rating, or use **Rating** in the right-click menu. Ratings are kept in each photo's
`.dvlp` file.

### Right-click menu

Right-click a photo in the filmstrip, or the photo in the viewer:

- **Export…** (or **Export N photos…** when several are selected)
- **Convert to DNG…** and **Convert to DNG, embedding the original…**
- **Copy settings** and **Paste settings** (to N photos)
- **Rating**
- **Show in File Explorer** (Windows) or **Show in Finder** (Mac): opens the photo's folder with
  the photo selected. Also in the grid and the People view.

When several photos are selected and you right-click one of them, the menu applies to all of them.

---

## 4. Looking at a photo

- **Zoom:** scroll the mouse wheel (or pinch on a trackpad) to zoom around the pointer.
  Double-click to switch between fit and 100%. The **View** section has **Fit**, **100%** and
  **200%** buttons, and the zoom readout in the viewer's corner switches between fit and 100%.
- **Pan:** drag the photo.
- **Rotate:** the two rotate buttons in the View section turn the photo a quarter turn left or
  right. Photos open upright as the camera recorded them.
- **A/B Split:** shows the photo before your edit on the left and after on the right, with a
  divider you drag. In the [Versions](#versions) section, **A/B** shows a saved version as the
  "before" instead.
- **Clipping:** marks blown highlights in red (every channel at maximum) or amber (some
  channels), and crushed blacks in blue.
- **Faces** (or **F**): outlines the faces found in the photo, with each person's name under
  their face ("Who is this?" for someone not named yet). Click a face (or its name) to name it:
  type the name and press **Enter**. Names you already use come up as you type; click one, or
  press **Tab** for the first. **Esc** closes the box.
  - A face nobody has named yet is grouped with faces that look like it, and naming it names
    the whole group, as naming them under [People](#people) does. The box says how many other
    faces that is.
  - A face that already has a name is being corrected: only that face moves to the new name.
    **Not (name)** takes the name off it; it is never put back with that person.
  - A photo that is not in your library is looked at for faces when you first show them (it
    takes a moment).
- **Histogram:** red, green and blue, white where all three overlap.
- **Vectorscope:** colours by hue (angle) and strength (distance from the centre); the amber line
  is skin tone.

---

## 5. Editing

Every edit is non-destructive: the original file is never changed. Edits are saved
automatically, a moment after you stop moving a slider, in a small `.dvlp` file beside the photo
(see [section 9](#9-where-your-edits-are-kept)).

The inspector's sections follow the order most photos are worked on. Slider scales follow
Lightroom: Exposure in stops (-5 to +5), most other sliders -100 to +100 with 0 meaning no
change.

### Presets

Save the current look and reuse it on other photos.

- **Save:** type a name and click **Save**. Tick **Include white balance** if the preset should
  set temperature and tint too (useful for photos shot in the same light). Saving under an
  existing name replaces that preset.
- **Apply:** click a preset's name. It applies to the open photo, or to every selected photo in
  the filmstrip.
- **Delete:** the **×** next to a preset.

A preset carries the profile, tone, presence, tone curve, color mixer or black and white mix,
and detail settings. Crop, rotation, lens correction, perspective (Transform), masks, retouching
and AI denoise stay as they are on each photo. Presets with white balance are marked **WB** in the list.

### Versions

Keep several versions of one photo's edit and switch between them.

- **Save:** type a name (or leave it empty for "Version 1", "Version 2"…) and click **Save**. A
  version keeps the whole edit, including crop, masks and retouching.
- **Switch:** click a version's name. **Undo** takes you back to the edit you had.
- **A/B:** shows that version as "before" in the A/B split, against your current edit.
- **Update:** replaces the version with the current edit.
- **×:** deletes the version.

Versions are kept in the photo's `.dvlp` file, so they travel with it, and appear in the
[filmstrip](#filmstrip). Versions made by an AI assistant are labelled `AI · ...` and
`Before AI · ...` (see [section 10](#10-ai-assistants)).

### Profile and treatment

- **Treatment:** **Color** or **Black & White**. Black & White turns the photo monochrome,
  replaces the Color Mixer with the [B&W Mix](#black-and-white-mix), and switches off Vibrance
  and Saturation.
- **Profile:** how the camera's colours and base tone are rendered:
  - **Adobe camera profiles** (Adobe Color, Adobe Standard and others) when Lightroom or Camera
    Raw is installed on the computer, including an approximation of Adobe Color.
  - **Standard (built-in):** Develop's own tone curve and colour.
  - **Linear:** no base curve: flat and dark, exactly what the sensor recorded.

  For JPEG, PNG, TIFF and HEIC pictures the profile is fixed ("Picture (sRGB)").

### Crop and straighten

Click **Crop & Straighten** (or press **R**). In crop mode:

- Drag the frame or its edges and corners to crop.
- Drag outside the frame to rotate (straighten); the **Angle** shows the result, up to ±45°. The
  crop shrinks to stay inside the turned photo and grows back when you turn back.
- Press **Enter**, double-click, or click **Done Cropping** (or **R** again) to finish.
- **Reset** clears the crop and the angle.
- **Ratio:** Free, Original, 1:1, 5:4, 4:3, 3:2, 7:5, 16:10, 16:9, 2:1, or one of your own.
  **Swap** turns the ratio between landscape and portrait. Under **New ratio**, enter a width and
  height and a name and click **Save** to keep it in the list; **Delete** removes the selected
  saved ratio.

The viewer shows the cropped size in pixels and the angle while you crop.

### White balance

- **Temp:** colour temperature in kelvin (2000-12000 K). Double-click its name for the
  as-shot value.
- **Pick** (or **W**): the white balance eyedropper. Click something in the photo that should be
  neutral grey or white, and temperature and tint are set to make it so. Esc cancels. Develop
  says so if the spot is too dark or too bright to judge.
- **As Shot:** the camera's own white balance.
- **5500 K:** daylight.
- **Auto:** Develop finds the white balance itself. When enough of the picture is grey or white,
  it makes those parts neutral: grey is the most reliable guide. When there is almost nothing
  grey, as under firelight, candles and tungsten, it goes by people's skin instead: it finds the
  faces, judges their skin as if the faces were correctly exposed, and cools the picture until the
  skin's colour is natural rather than orange, stopping before skin turns pink. That is what makes
  those photos come out right, where the camera leaves everything orange. With neither grey nor
  faces it changes nothing. Faces lit by strongly coloured light (a blue or purple stage or LED
  light) are left alone too, with a note saying so: no white balance can make that skin natural.
  Only temperature (and tint, from grey areas) is changed, never exposure; a note under the button
  says what it went by, and **Undo** goes back.
- **Skin:** goes by the people only, whatever the rest of the picture shows, for when you care
  more about how the people look than about the walls (a face in shade beside a sunlit white
  wall, say). Temperature sets how strongly coloured the skin is (cooling it when the light is
  orange, never warming pale skin), and tint turns its hue to natural (a little green when skin
  looks pink, a little magenta when it looks green-yellow). It needs a face; without one, or under
  strongly coloured light, it changes nothing and says why. Exposure is never changed, and
  **Undo** goes back.
- **Tint:** -150 (green) to +150 (magenta), relative to the as-shot white balance.

### Tone

- **Exposure:** overall brightness, in stops.
- **Contrast**
- **Highlights:** brings bright areas down (or up) while keeping their texture.
- **Shadows:** opens dark areas (or deepens them).
- **Whites** and **Blacks:** set the brightest and darkest points.

Highlights, Shadows and Whites adapt to each photo's own brightness, as Lightroom's do.

### Presence

- **Texture:** fine detail and surface texture (negative values soften, for example skin).
- **Clarity:** local contrast in the midtones.
- **Dehaze:** removes (or adds) haze.
- **Vibrance:** strengthens muted colours more than strong ones, and protects skin tones.
- **Saturation:** strengthens or weakens every colour equally (-100 is grey).

### Tone curve

- **The graph:** click the curve to add a point, drag points to shape it, double-click a point to
  remove it (the end points stay). The readout below shows the point's input and output on
  Lightroom's 0-255 scale.
- **Highlights, Lights, Darks, Shadows:** push regions of the tone range up or down. They are
  fitted to Lightroom's own (Darks reaches across most of the range, for example).
- **Reset:** a straight curve and every region at 0.

The curve is applied so colours keep their hue (a sunset stays orange when its highlights come
down). Measured against Lightroom, a curve lands within about 1-2 levels of Lightroom's.

### Color mixer

Hue, saturation and luminance (brightness) for eight colours: red, orange, yellow, green, aqua,
blue, purple and magenta. Choose **Hue**, **Saturation** or **Luminance** at the top; **Reset**
puts that tab's sliders back to 0.

- **Hue** moves a colour towards its neighbour (red towards orange or magenta, for example).
- **Saturation** -100 turns a colour grey at its own brightness; +100 roughly doubles it.
- **Luminance** brightens or darkens a colour without changing its hue.

It works on the colours as you see them on screen, and leaves greys alone. Orange covers skin,
sand and warm light; blue covers most skies; green and yellow cover foliage.

### Black and white mix

Shown instead of the Color Mixer when the treatment is **Black & White**: one slider per colour
sets how bright that colour turns out in grey. For example, lower **Blue** for a darker sky,
raise **Orange** for brighter skin or sand, raise **Green** and **Yellow** for lighter foliage.
**Reset** puts every colour back to 0.

### Detail

- **AI Denoise:** cleans sensor noise with the RawNIND model. It works on raw files (including
  Fujifilm X-Trans) and on phone DNGs; switching it on or off opens the photo again, which takes
  a moment (progress shows in the inspector). On Windows it runs on the graphics card through
  DirectML; on a Mac, on the processor (Bayer raws) or the graphics card (other files).
- **Sharpening:** amount (0-150; 40 by default for raw files).
- **Radius:** the size of the detail sharpened (0.5-3 pixels).
- **Detail:** how much fine detail is sharpened (0-100).
- **Masking:** leaves smooth areas such as sky and skin unsharpened (0-100).

Sharpening is shown as it will look at the current zoom, so fit view looks like a resized export.

**Noise Reduction** works on every photo, JPEG, PNG, TIFF and HEIC included, live as you drag:

- **Luminance** (0-100): smooths grain in brightness. Single bright or dark specks are removed
  first.
  - **Detail** (50 by default): higher keeps more fine detail; lower smooths more.
  - **Contrast** (0 by default): gives back texture that is bigger than the grain.
- **Color** (0-100): removes colour blotches, keeping colours from bleeding across edges.
  - **Detail** (50 by default): higher keeps small coloured details.
  - **Smoothness** (50 by default): higher evens out wider blotches.

Detail, Contrast and Smoothness are greyed out until their slider above is above 0. A trace of
fine grain is kept, so smooth areas don't look plastic. For raw files, **AI Denoise** above is
usually cleaner still; the two can be used together. Noise reduction is kept in presets and
versions.

### Lens correction

- **Enable Profile Corrections** with a **Source**:
  - **Auto:** a Lensfun profile for the lens when there is one, otherwise the camera's embedded
    corrections if the file has them (Sony).
  - **Lensfun:** a lens profile from the Lensfun database, which comes with Develop. The lens is
    matched automatically; pick another with **Make**, **Model** and **Profile**.
  - **Camera embedded corrections:** the correction data in the raw file.
- **Profile Distortion** and **Profile Vignetting:** how much of the profile's correction is
  applied (100% by default, up to 200%).
- **Constrain Crop:** enlarges just enough that corrections never show empty borders.
- **Distortion:** manual barrel or pincushion correction.
- **Defringe:** **Purple Amount** and **Purple Hue**, **Green Amount** and **Green Hue** remove
  coloured fringes on high-contrast edges.
- **Red / Cyan** and **Blue / Yellow:** correct colour fringes towards the corners (lateral
  chromatic aberration).
- **Vignette:** **Amount** darkens or lightens the corners, **Midpoint** sets how far in it reaches.

For Nikon Z, Panasonic, Olympus / OM System and Fujifilm cameras, distortion correction is
applied automatically even with profile corrections off, as Lightroom does for these cameras.
To add lenses, open the Lensfun folder from the panel, drop Lensfun `.xml` files in, and reload.

### Transform

Straightens converging lines: buildings that lean in because the camera was pointed up, a wall
photographed at an angle, a horizon that tilts.

- **Upright** finds the long straight lines in the photo and sets the sliders below for you:
  - **Level:** only turns the photo, so level lines lie level.
  - **Vertical:** turns it and corrects **Vertical**, so upright lines (buildings, poles, door
    frames) stand straight.
  - **Full:** also corrects **Horizontal**, so level lines straighten too.
  - **Auto:** a balanced version of Full that holds back where a strong correction would distort
    the photo.
- **Guided:** draw the lines yourself. Click **Guided**, then drag along two to four things that
  should be upright or level (the edge of a building, a door frame, the horizon). Each line you
  draw turns the photo to match: two upright lines set **Vertical**, two level lines set
  **Horizontal**, and any line sets the angle. Right-click a line to remove it; **Clear Guides**
  removes them all; **Done** (or **Esc**) when finished. The lines are not kept with the photo.
- The sliders, which you can also move yourself or after Upright:
  - **Vertical:** tilts the photo forward or back. Move it left when buildings lean in at the
    top.
  - **Horizontal:** turns the photo left or right, for a wall seen at an angle.
  - **Rotate:** the same angle as in [Crop and straighten](#crop-and-straighten).
  - **Aspect:** stretches the photo wider (right) or taller (left), when a strong correction
    makes things look squashed.
  - **Scale:** makes the corrected photo bigger or smaller.
  - **X Offset** and **Y Offset:** move it.
- The crop always stays inside the corrected photo, so no empty corners show (it gets smaller as
  the correction gets stronger). **Crop & Straighten** shows the whole corrected photo.
- **Grid** draws lines over the photo, to judge what is straight. **Reset** puts every slider
  back, the angle too.

The correction uses the focal length the photo records, so it bends lines as a differently
pointed camera would have.

### Masks

Masks are sliders that change only part of the photo: you paint where they apply. Darken a bright
sky, lift a face in shadow, warm the sand, add clarity to a rock, all without touching the rest.

- Click **Brush** (or press **K**) and paint on the photo. The first stroke starts **Mask 1**.
  Click again (or press **Esc**) when you are done.
- **+ New** starts another mask, with its own sliders. A photo can have up to 8 masks. Click a
  mask's name in the list to select it: painting and the sliders below act on the selected mask.
- Each mask has **Temp**, **Tint**, **Exposure**, **Contrast**, **Highlights**, **Shadows**,
  **Whites**, **Blacks**, **Texture**, **Clarity**, **Dehaze** and **Saturation**. They add to
  the photo's own sliders where the mask is painted: Exposure −1 in a mask makes that part one
  stop darker than the rest. Where two masks overlap, both apply. **Reset** puts the selected
  mask's sliders back to 0.
- **Brush:** **Add** paints into the mask, **Erase** takes parts out; hold **Alt** to paint
  with the other one. **Size**, **Feather** (soft edge) and **Flow** (how much one stroke paints
  in; paint over a place again to build it up). **[** and **]** make the brush smaller or bigger,
  **Shift+[** and **Shift+]** change the feather.
- **Auto Mask** keeps the brush to areas that look like the one under its centre (the small
  cross), so it stops at edges: paint along a skyline with the cross in the sky and the trees or
  roofs below are left out. Keep the centre on what you want: where it passes over something
  else, that is painted in instead.
- **Show Overlay** (or **O**) shows the selected mask in red while the brush is on.
- The tick beside a mask switches it off and on, to compare; the **×** deletes it. You can rename
  the selected mask under the list.
- **Space**+drag (or the middle mouse button) pans while the brush is on.

Masks are kept with the photo's other edits, in versions, and in Undo; they are copied with
**Copy settings**. Presets leave them alone.

### Retouch

Remove dust spots, blemishes and small distractions. Click **Remove Spots** (or press **Q**):

- **Click** a spot to remove it with a circle, or **drag** to paint over a longer one (a wire, a
  branch, a scratch). Develop picks a nearby area to copy from.
- **Mode:** **Heal** blends the copied area into its surroundings; **Clone** copies it exactly.
- Drag either outline to move it (the target or its source). The two are joined by a line when
  selected.
- **Size**, **Feather** and **Opacity** apply to the selected spot, or to new spots when none is
  selected.
- **[** and **]** make the brush (and the selected spot) smaller or bigger; **Shift+[** and
  **Shift+]** change the feather.
- **Delete** (or Backspace) removes the selected spot; **Clear** removes all.
- **H** hides the outlines; **Space**+drag (or the middle mouse button) pans; **Esc** deselects,
  then leaves the tool.

Spots are layered in order: a Heal painted after a Clone copies from, and blends into, the cloned
result, so you can clone out an awkward edge first and heal over it. Healing is worked out across
the whole painted shape, so wide patches blend smoothly. A photo can have up to 64 spots.

---

## 6. Undo, reset, copy and paste

- **Undo** and **Redo**: **Ctrl+Z**, and **Ctrl+Shift+Z** or **Ctrl+Y** (Cmd on a Mac), or the
  buttons at the top of the inspector. Each photo has its own history while Develop is open; one
  slider drag is one step.
- **Reset All:** the whole edit back to Develop's defaults: every slider, white balance (as
  shot), crop and angle, perspective (Transform), tone curve, color mixer, black and white,
  sharpening, lens correction, masks and retouching. Undo brings it back. Versions and the rating are kept.
- **Copy and paste settings:** **Ctrl+C** copies the open photo's settings (also from the
  right-click menu); **Ctrl+V** pastes them onto the open photo, or onto every selected photo.
  Pasting onto photos that are not open keeps each photo's own crop, rotation and perspective. Copying also
  puts a line of text on the system clipboard, which is what makes Ctrl+V reliable on Windows.

---

## 7. Exporting

Click **Export…** at the bottom of the inspector, press **Ctrl+E**, or right-click photos and
choose **Export…**. Set the options below, then click the orange **Export…** button at the bottom
of the dialog and choose where to save. With several photos selected, each is exported with its
own settings into a folder you choose, with a progress bar. When it is finished, Develop says so;
**Show in folder** opens that folder, with the exported photo selected.

**Preset:** keep the export settings below under a name (**Save**), pick one, or **Delete** it.

**Format**

| Format | Options |
|---|---|
| JPEG | **Quality** 1-100 |
| PNG | **Bit depth** 8 or 16 bits |
| TIFF | **Bit depth** 8 or 16 bits |
| DNG (original raw) | The original raw data, not a rendered picture: see [section 8](#8-converting-to-dng). **Embed the original raw file** stores the raw file inside the DNG too. |

**Colour space:** sRGB.

**Size:** Original size, or resize by Long edge, Short edge, Width, Height (pixels),
Megapixels, or Percentage. **Don't enlarge** never makes a picture bigger than the original.
**Resolution** (pixels per inch) is a print-size hint written into the file; it does not change
the pixels.

**Metadata**

- **All:** camera, lens, exposure, capture date, artist and copyright. **Remove location** leaves
  the GPS position out.
- **Copyright only:** artist and copyright.
- **None:** no metadata at all.
- **Artist** and **Copyright:** text written into the file (filled in from the camera when it
  recorded them).

TIFF files carry the camera, date, artist and copyright only.

---

## 8. Converting to DNG

DNG is Adobe's open raw format. Converting saves the original raw data, losslessly compressed,
for editing in other apps or for archiving.

- Right-click raw photos and choose **Convert to DNG…** (or **…embedding the original…** to store
  the original raw file inside the DNG as well, about twice the size), or choose **DNG (original
  raw)** as the format in the Export dialog.
- Develop writes the DNG itself. It keeps the camera and lens details (all of the original's
  EXIF), the camera's baseline exposure, and, for cameras whose distortion Lightroom always
  corrects (Nikon Z, Panasonic, Olympus / OM System, Fujifilm), that correction in a standard form
  other DNG readers apply too.
- Develop's edits come along in a `.dvlp` file beside each DNG, so Develop reopens it as you left
  it. Other apps open the DNG unedited.
- The originals are never changed or deleted.

---

## 9. Where your edits are kept

**Beside each photo:** `DSC01234.ARW.dvlp`, a small text file with the photo's edit, rating,
saved versions, keywords and the names of the people in it. Only photos with any of these get one. Keep it with the photo when you copy or move
it, and the edit comes along. If a folder cannot be written to (a memory card, a read-only disk),
the edit is kept in the settings folder instead.

**Changes made by another program show at once.** While a folder is open, Develop is told when
another program changes one of its `.dvlp` files: an AI assistant editing through Develop's MCP
server, or a second copy of Develop. New ratings and versions appear in the filmstrip, and if the
changed photo is the one you have open, its sliders and picture update and a note says *Edited by
another program*; **Undo** brings back what you had. The newest change wins, so neither program
saves an old copy over the other's. (Develop does not check the folder on a timer for this; the
system tells it. On a shared folder on another computer it checks every few seconds instead.)
Other photos' thumbnails show the camera's preview until you open them again.

**The settings folder:** `%APPDATA%\Develop` on Windows, `~/Library/Application Support/Develop`
on a Mac.

| File | Holds |
|---|---|
| `develop-presets.tsv` | Your develop presets |
| `export-settings.tsv` | Last export settings and export presets |
| `crop-presets.tsv` | Your saved crop ratios |
| `session.tsv` | The folder and photo to reopen |
| `network.tsv` | Computers and shares added to Network |
| `photo-settings.tsv` | Edits for photos in folders that cannot be written to |
| `update-skip.txt` | A version you chose to skip |
| `layout.tsv` | Panel sizes, the left panel's tab, the grid view and its thumbnail size |
| `sections.tsv` | Which sections you closed (the others are open) |
| `library.txt` | The library's folders |
| `library-index.tsv` | What Develop knows about each library photo, for searching |
| `library/` | The faces found in library photos (where they are, and a fingerprint of each), the people they are grouped into, and what each photo shows (for searching by description and keyword suggestions): one file per photo folder and per named person |
| `faces.tsv`, `people.tsv`, `clip.tsv` | The library as Develop 0.4 kept it, left as it was so an older Develop still works |
| `dates/` | Capture dates of folders' photos, so folders open in order at once |
| `faces-paused` | There while finding faces is paused |
| `moves.log` | A record of folders moved with Move to… |
| `freeze.log` | Notes made when the window stops responding for a few seconds: what Develop was doing (see Troubleshooting) |
| `keyword-vocabulary.txt` | Your own list of words for keyword suggestions, if you make one |
| `thumbs/` | Thumbnails of edited photos and of versions, and small pictures of faces |

A [shared library](#sharing-a-library) keeps its face and search data in a hidden `.develop`
folder inside the shared folder instead of `library/`.
| `lensfun/` | Your extra Lensfun lens profiles |
| `mcp-http.txt` | The address and token of the HTTP MCP server, when used |

Earlier versions of Develop kept their settings under `SonyRawProcessor`; they are copied over
the first time a newer version starts.

---

## 10. AI assistants

Develop includes an MCP server, so an AI assistant (Claude, Gemini, ChatGPT-based tools, local
models in LM Studio, Cursor, VS Code Copilot, Codex, Continue and others) can help with your
photos: list a folder, read and change edits, measure a photo with numbers (histogram, clipping,
colour cast) or look at a preview, apply presets, rate, save and restore versions, export, and
convert to DNG.

Develop protects your work while an assistant edits:

- Your current edit is kept first as a version named `Before AI · <time>`.
- Each request's changes to a photo go into one version named after the app and what changed,
  for example `AI · Claude: exposure, highlights · 17 Nov 14:32 UTC`. Alternatives the assistant
  makes on request (a black and white take, say) get their own version.
- Versions you made can never be changed or removed by an assistant.

An assistant can also search your library (by keyword, person, file name or description), add
keywords, suggest keywords from what is in a photo, and list the people you have named.

Setup for each AI app, every tool and setting, good practice and privacy notes are in
**[MCP.md](MCP.md)** (including `check_photos`, which lets an assistant measure a whole folder at once; focus checks that measure the full-size photo and show the assistant its sharpest area at 100%; `cull_photos`, which rates a whole shoot by focus, keeping the sharpest frame of each burst; and a skin-tone reading, so an assistant can judge white balance from people's faces in firelight or tungsten, where nothing in the picture is grey), and a ready-made guide to give your assistant is in
**[develop-skill](develop-skill/SKILL.md)**.

---

## 11. Keyboard and mouse

Ctrl means Cmd on a Mac. Keys work when no text box has the keyboard.

| Key | Action |
|---|---|
| Ctrl+O | Open a file |
| ← / → | Previous / next photo in the filmstrip |
| 1-5, 0 | Rate the selected photos (0 removes the rating) |
| Ctrl+C / Ctrl+V | Copy / paste settings |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z or Ctrl+Y | Redo |
| Ctrl+E | Export |
| R | Crop and straighten on / off |
| Enter | Finish cropping |
| W | White balance eyedropper (Esc cancels) |
| F | Show / hide the faces on the photo, to name them |
| Q | Retouch (spot removal) on / off |
| K | Mask brush on / off |
| O | With the mask brush: show / hide the red overlay |
| Alt (held) | With the mask brush: paint with the other mode (erase while adding) |
| [ / ] | Show / hide the folder panel / inspector; in Retouch or with the mask brush, brush smaller / bigger |
| Shift+[ / Shift+] | In Retouch or with the mask brush: feather harder / softer |
| Delete, Backspace | In Retouch: remove the selected spot |
| H | In Retouch: hide / show the outlines |
| Space+drag | In Retouch or with the mask brush: pan |
| Esc | In Retouch: deselect, then leave the tool; with the mask brush or Guided: leave it; in the People view: back to the photo |
| Delete | In the People view: take the picked photos out of the set |

| Mouse | Action |
|---|---|
| Scroll / pinch | Zoom around the pointer |
| Drag | Pan (in Retouch: paint, or move an outline; with the mask brush: paint; in Guided: draw a line) |
| Double-click the photo | Fit / 100% (in crop mode: finish) |
| Middle-button drag | Pan (also in Retouch and with the mask brush) |
| Double-click a slider's name | Back to its default |
| Right-click a photo | Export, Convert to DNG, Copy / Paste settings, Rating, Show in File Explorer / Finder |
| Right-click a folder | Add to / Remove from Library, Move to… |
| Ctrl-click / Shift-click in the filmstrip, grid or People view | Add to the selection / select a range |
| Double-click a photo in the grid or People view | Open it |
| Drag photos from the People view onto a face under People | Move them to that person |
| Drag panel edges, the filmstrip's top edge | Resize them |

---

## 12. Supported files and cameras

**Raw files:** every format the rawler decoder reads, around 825 camera models: ARW, CR2, CR3,
CRW, CRM, NEF, NRW, RW2, RWL, RAF (including X-Trans), ORF, ORI, PEF, SRW, IIQ, 3FR, FFF, DCR,
DCS, KDC, MEF, MOS, MRW, ERF, X3F, ARI, QTK, RAW and DNG (including phone DNGs).

**Pictures:** JPEG, PNG, TIFF and HEIC, edited in linear light so the same sliders work on them.

**Calibrated against Lightroom:** Sony, Nikon, Canon, Panasonic and Fujifilm. Sliders, white
balance and Lightroom's per-camera adjustments (built-in lens corrections, exposure offsets) were
measured on reference photos from these brands' cameras; on 18 reference photos a slider lands
within about 4 levels (out of 255) of Lightroom on average.

**Supported, not yet calibrated:** every other camera the decoder reads (Olympus / OM System,
Pentax, Leica, Samsung, Hasselblad, Phase One, Sigma, Kodak, Minolta, Epson, phones). They open
and edit normally, with colour from the camera's own data (and Adobe's profile when Lightroom or
Camera Raw is installed); a slider may look a little different from Lightroom.

---

## 13. Command line

For scripting and automation. On Windows the program is
`%LOCALAPPDATA%\Develop\develop.exe`; on a Mac,
`/Applications/Develop.app/Contents/MacOS/develop`.

| Command | Does |
|---|---|
| `develop <photo>` | Open Develop on a photo |
| `develop <photo> --export <out.jpg> [options]` | Export one photo and quit, without opening a window (format from the file extension: `.jpg`, `.png`, `.tif`) |
| `develop <first> [options] --batch-export <folder> <more photos…>` | Export several photos into a folder and quit |
| `develop --to-dng <folder> [--embed] <raw files…>` | Convert raw files to DNG in a folder |
| `develop --mcp` | MCP server on standard input / output (see [MCP.md](MCP.md)) |
| `develop --mcp-http <port> [--mcp-token <token>]` | MCP server over HTTP on this computer |

Export options: `--export-long <pixels>`, `--export-percent <percent>`, `--export-quality <1-100>`,
`--export-bits 16`, `--export-metadata <0 none, 1 copyright, 2 all>`, and `--with-settings 1` to
use the photo's saved edit (without it a command-line export uses Develop's defaults).
Other `--name value` options set sliders for testing.

---

## 14. Troubleshooting

- **A photo opens dark, flat or with odd colours.** Check the **Profile**: Linear is flat and dark
  on purpose; pick an Adobe profile or Standard.
- **Colours differ slightly from Lightroom.** Expected for cameras outside the five calibrated
  brands, and when Adobe's camera profiles are not installed (install Lightroom or Camera Raw to
  get them).
- **AI Denoise is greyed out.** It works on camera raw files and phone DNGs, not on JPEG, PNG,
  TIFF or HEIC. If it says the model files were not found, reinstall Develop.
- **Ctrl+V does nothing.** Copy the settings with Ctrl+C in Develop first (that also puts text on
  the clipboard Ctrl+V needs), or use the right-click menu.
- **Edits did not follow a copied photo.** Copy its `.dvlp` file along with it.
- **Develop stopped responding.** Whenever the window stops responding for more than a few
  seconds, Develop writes down what it was doing in `freeze.log` in the settings folder (times
  in UTC), and notes when it responds again. If it happens, send that file along with what you
  were doing: it shows the step it was stuck on.
- **A network computer is missing.** Add it by address in Network; on Windows, make sure network
  discovery is on; on a Mac, use Connect….
- **Changes made by an AI assistant do not show.** They normally appear within a second. On some
  network shares Develop is not told about changes and checks every few seconds instead; wait a
  moment, or open the folder again.
- **The People list is empty, or library search finds nothing.** Add folders to the library
  (Faces & Metadata, **+ Add**) and wait for **Finding faces** to finish.
- **No suggested keywords, or search finds nothing by description.** The image understanding
  model files were not found; reinstall Develop.
- **The same person is split into two groups.** Give both the same name, or drag the photos onto
  the right face; check **Possible matches** too. Once a few people are named, **Regroup** moves
  the clear matches for you.
- **Wrong faces in someone's set.** On their page, choose **Show: Least alike first** and take out
  the ones that aren't them.
- **Finding faces makes the computer busy.** Click **Pause**; it stays paused until you resume it.
- **A move stopped part-way.** Run **Move to…** again with the same destination: it checks what
  is already there and carries on. Nothing was removed that wasn't safely copied.
- **A shared library doesn't show the other computer's changes.** Check that both computers can
  reach the folder and run Develop 0.5.0 or later; changes show within seconds.
- **Graphics problems.** The top bar shows which graphics system Develop uses; updating the
  graphics driver usually helps.
- **macOS says the app is damaged or will not open.** See the README's
  [Installing on macOS](../README.md#installing-on-macos).

---

## 15. Privacy

Develop has no account, no cloud and no telemetry. It works on the files on your computer and
network. It connects to the internet only to check GitHub for updates when it starts. Its own AI
(noise reduction, face recognition, and reading what is in photos for keywords and search) runs
entirely on your computer; face fingerprints stay in the settings folder and are never written
into your photos or sent anywhere. An AI assistant connected through MCP sees only
what you point it at; a cloud assistant receives the photo previews it asks for, as anything you
paste into its chat would (see [MCP.md](MCP.md#privacy-and-safety)).

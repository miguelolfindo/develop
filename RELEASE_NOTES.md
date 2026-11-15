## Develop 0.2.0

### New cameras
- **Nikon NEF**, **Canon CR2 and CR3**, **Panasonic RW2** and **Fujifilm RAF** files now
  open, alongside Sony ARW and DNG.
- **Fujifilm X-Trans** sensors get their own demosaicing, and AI noise reduction works on
  them too.
- Lightroom's per-camera adjustments are matched: built-in distortion correction for
  Nikon Z, Panasonic, Olympus / OM System and Fujifilm cameras, exposure offsets for some
  Panasonic models, and Fujifilm's DR200 / DR400 exposure compensation.

### Closer to Lightroom
The sliders were refitted against Lightroom Classic exports of 18 photos from five camera
brands (daylight, shade, indoor, backlit and night scenes):
- **Shadows, Whites, Highlights, Clarity and Dehaze adapt to each photo** the way
  Lightroom's do: the same slider value does more on a dark, low-key photo and less on a
  bright one.
- **Contrast** now uses Lightroom's -100 to +100 scale.
- **Tint** moves the white point the way Adobe defines it, so magenta and green shifts
  look right under warm and cool light alike.
- **Texture, Clarity and Dehaze** were fitted to Lightroom, and Clarity no longer leaves a
  dark halo along dark edges against a bright sky.

On average, a slider in Develop now lands within about 4 levels (out of 255) of the same
slider in Lightroom.

### Lens correction
- The **Lensfun lens database** is now included, so lens profiles work without adding
  anything.
- Better lens matching: no more wrong guesses for cameras that did not record their lens.

### Also new
- `lightroom/DevelopReferences.lrplugin`: a Lightroom Classic plugin that exports the
  reference JPEGs used to tune Develop.
- Develop is now licensed under the GNU GPL v3 or later.

### Installing
- **Windows:** run `develop_0.2.0_x64-setup.exe`. It installs over an earlier version and
  keeps your settings and sidecar files.
- **macOS (Apple silicon, macOS 13.4 or later):** open `Develop_0.2.0_aarch64.dmg` and
  drag Develop to Applications. The app is not notarized: the first time, right-click
  Develop in Applications, choose **Open**, then **Open** again.

### Known limitations
- Dehaze is the slider furthest from Lightroom: its negative values add an even veil,
  where Lightroom's varies across the frame.
- Panasonic models other than the G9, GH6 and S5 do not have their exposure offset
  measured yet and may render slightly brighter or darker than in Lightroom.

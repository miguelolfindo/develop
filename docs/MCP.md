# Develop and AI assistants (MCP)

Develop includes a [Model Context Protocol](https://modelcontextprotocol.io) (MCP) server. Any
AI assistant that speaks MCP (Claude, ChatGPT-based tools, Gemini, local models in LM Studio,
coding assistants such as Cursor, VS Code Copilot, Codex or Continue) can then help with your
photo work: look through a folder, rate, apply a look, match a series, make black & white
versions, export.

The assistant works the way you do in Develop: it reads and writes each photo's `.dvlp`
edit file, and never changes the original raw files. You decide which AI to connect and
which folders to point it at.

**Every AI request becomes its own version.** Develop enforces this itself, whatever the AI is
told (see [AI versions](#ai-versions)): your edit is kept, the AI's result is saved as one
clearly labelled new version per photo per request, and versions you made are never changed
or removed.

- [How it works](#how-it-works)
- [AI versions](#ai-versions)
- [Connecting an AI app](#connecting-an-ai-app)
- [Connecting over the network](#connecting-over-the-network)
- [Teaching the assistant: the Develop skill](#teaching-the-assistant-the-develop-skill)
- [Tools](#tools)
- [Settings reference](#settings-reference)
- [Checking an edit with numbers](#checking-an-edit-with-numbers)
- [Good practice](#good-practice)
- [Privacy and safety](#privacy-and-safety)
- [Troubleshooting](#troubleshooting)
- [Protocol details](#protocol-details)

## How it works

Develop's program doubles as the server. Started with one of these arguments it does not
open a window; it waits for an AI app to talk to it:

| Command | For |
|---|---|
| `develop --mcp` | AI apps on your computer that start the server themselves (most of them). They talk to it over standard input and output. |
| `develop --mcp-http <port>` | AI services that connect over the network. Listens on this computer only and requires an access token. |

Where the program is:

- **Windows:** `C:\Users\<you>\AppData\Local\Develop\develop.exe`
- **macOS:** `/Applications/Develop.app/Contents/MacOS/develop`

You do not need to start anything by hand for the first kind: the AI app starts Develop when it
needs it and stops it afterwards. Develop itself can be open or closed meanwhile. If it is open
on the same folder, you see the assistant's work as it happens: its ratings and versions appear
in the filmstrip, and the photo you have open updates when the assistant edits it (Undo goes
back). Edits you make in Develop meanwhile are read by the server before it changes that photo,
so neither side saves an old copy over the other's.

To measure or preview a photo, the server renders it with Develop's own engine on the graphics
card, without opening a window, so nothing appears on screen or in the taskbar while an
assistant works.

## AI versions

A hard rule, built into the server so that no assistant can skip it. Whenever an AI changes a
photo (`set_settings`, `apply_preset`, `reset_settings`, `restore_version`):

1. **Your edit is kept first.** At the start of a request, if the photo's current edit is not
   already saved as one of its versions, it is saved as **`Before AI · 17 Nov 14:32 UTC`**.
2. The change is made.
3. **One new version per photo per request.** The request's first change to a photo makes a new
   version named after the app and what changed; further changes in the same request update
   that same version, and its name gathers everything changed, for example
   **`AI · Claude: exposure, highlights, shadows · 17 Nov 14:32 UTC`** or
   **`AI · LM Studio: preset Beach wedding · 17 Nov 14:40 UTC`**. So however many small steps the
   assistant takes while working, you get one version to look at.

**What counts as a request.** Develop cannot see your chat, only the assistant's tool calls. An
assistant works through a request with calls in quick succession; a pause of five minutes with
no calls at all (while you read its answer and write the next message), or the assistant
reconnecting, starts the next request.

**Alternatives get their own version.** When you ask for a different take ("also make a black
and white version"), the assistant passes `new_version: true` and the alternative is saved
separately instead of replacing the first result.

Versions the AI saves itself with `save_version` are always named **`AI · ...`** too.

No tool can change or delete an existing version: the only version an assistant's edits ever
update is the one its current request made. Versions you made, and earlier requests' versions,
are never overwritten.
In Develop, AI versions appear in the Versions section and in the filmstrip (V1, V2...) under
their labels: click one to look at it, switch back to yours, or delete the ones you do not want.

If a request has a mistake in it (an unknown setting, say), no photo is changed and no version
is made.

## Connecting an AI app

Replace `<you>` with your Windows user name. In JSON files, backslashes in Windows paths are
written twice (`\\`). On a Mac, use the macOS path above instead.

### Claude Desktop

Settings › Developer › Edit Config opens `claude_desktop_config.json`. Add:

```json
{
  "mcpServers": {
    "develop": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Develop\\develop.exe",
      "args": ["--mcp"]
    }
  }
}
```

Restart Claude Desktop. The Develop tools appear in the tools menu of a new chat.

### Claude Code

```bash
claude mcp add develop -- "C:\Users\<you>\AppData\Local\Develop\develop.exe" --mcp
```

### LM Studio (local models)

LM Studio 0.3.17 and later: Program › Install › Edit `mcp.json` (it uses the same layout as
Cursor), and add inside `mcpServers`:

```json
{
  "mcpServers": {
    "develop": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Develop\\develop.exe",
      "args": ["--mcp"]
    }
  }
}
```

Use a model that supports tool calling, and give it the [Develop skill](#teaching-the-assistant-the-develop-skill)
as its system prompt: small local models do much better with it. To let it look at previews,
the model must also accept images.

### Cursor

`~/.cursor/mcp.json` (all projects) or `.cursor/mcp.json` (one project):

```json
{
  "mcpServers": {
    "develop": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Develop\\develop.exe",
      "args": ["--mcp"]
    }
  }
}
```

### VS Code (GitHub Copilot agent mode)

`.vscode/mcp.json` in a folder, or your user `mcp.json` (Command Palette › MCP: Open User
Configuration). VS Code calls the list `servers` and wants a `type`:

```json
{
  "servers": {
    "develop": {
      "type": "stdio",
      "command": "C:\\Users\\<you>\\AppData\\Local\\Develop\\develop.exe",
      "args": ["--mcp"]
    }
  }
}
```

### Gemini CLI

`~/.gemini/settings.json` (or `.gemini/settings.json` in a project):

```json
{
  "mcpServers": {
    "develop": {
      "command": "C:\\Users\\<you>\\AppData\\Local\\Develop\\develop.exe",
      "args": ["--mcp"],
      "timeout": 120000
    }
  }
}
```

The longer `timeout` (milliseconds) gives exports of many photos time to finish.

### OpenAI Codex CLI

`~/.codex/config.toml`:

```toml
[mcp_servers.develop]
command = 'C:\Users\<you>\AppData\Local\Develop\develop.exe'
args = ["--mcp"]
```

(Single quotes in TOML keep the backslashes as they are.)

### Continue

In `config.yaml`:

```yaml
mcpServers:
  - name: Develop
    command: C:\Users\<you>\AppData\Local\Develop\develop.exe
    args:
      - --mcp
```

### Any other MCP app

Look for "MCP servers", "tools" or "connectors" in its settings. Whatever the layout, you need
three things: the **command** (the path to `develop.exe` or the macOS `develop` above), the
**arguments** (`--mcp`), and the **transport** (`stdio`, also called "command" or "local").

## Connecting over the network

For an AI that runs somewhere else (a web service, another computer, a home server running
Open WebUI), start the HTTP server in a terminal:

```bash
develop --mcp-http 8765
```

It prints the address and an access token:

```
Develop MCP server on http://127.0.0.1:8765/mcp
Access token: 3f9c...e21a
```

The same two lines are saved as `mcp-http.txt` in Develop's settings folder
(`%APPDATA%\Develop` on Windows, `~/Library/Application Support/Develop` on a Mac). Choose
the token yourself with `--mcp-token <token>`.

Every request must carry the token as an HTTP header:

```
Authorization: Bearer <token>
```

Apps that take a remote MCP server usually ask for a URL and let you add headers. In the
layout most of them share:

```json
{
  "mcpServers": {
    "develop": {
      "url": "http://127.0.0.1:8765/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

(Gemini CLI calls the field `httpUrl` instead of `url`.)

**Reaching it from outside your computer.** The server only accepts connections from this
computer. To let a cloud service reach it you need a tunnel or reverse proxy you control
(for example Cloudflare Tunnel, Tailscale Funnel, ngrok or a proxy on your own server) that
forwards HTTPS to `127.0.0.1:8765`. Keep the token secret: whoever has it can change your
edits and export your photos. Stop the server (Ctrl+C) when you are done.

**Services that cannot send a token.** Some web assistants only connect to MCP servers that
sign in with OAuth or need no sign-in at all. Develop's HTTP server does neither, on purpose:
putting your photo library on the internet without a password is not safe. Use one of the
local apps above instead, or a proxy that adds the header for you and is itself protected.

## Teaching the assistant: the Develop skill

The server tells every assistant the basics when it connects (slider scales, saving a version
first), and every tool describes itself. For better results, give the assistant the **Develop
skill**: a short guide to editing workflows, judging previews and working safely, in
[`docs/develop-skill/SKILL.md`](develop-skill/SKILL.md).

- **Claude (Desktop, Code, claude.ai):** use it as a skill: copy the `develop-skill` folder
  into your skills folder (Claude Code: `~/.claude/skills/`), or upload it under Settings ›
  Capabilities › Skills.
- **ChatGPT, Gemini, LM Studio and most others:** paste the text (everything below the
  `---` header) into the assistant's custom instructions, system prompt or project
  instructions.
- **Coding assistants (Codex, Cursor, Copilot, Continue):** save it as `AGENTS.md` (or the
  app's rules file) in the folder you work in.

## Tools

Every tool that takes photos accepts either `path` (one photo) or `paths` (a list). Paths
are full paths to the photo files. Results come back as JSON text, except `render_preview`,
which returns an image.

| Tool | What it does | Main arguments |
|---|---|---|
| `list_photos` | The photos in a folder, with rating, whether edited, and how many versions | `folder`, `min_rating` (0-5) |
| `get_settings` | One photo's whole edit, on the sliders' own scales | `path` |
| `set_settings` | Change some settings on one or more photos; anything not named stays as it is | `path`/`paths`, `settings`, `new_version` |
| `reset_settings` | Back to Develop's defaults (ratings and versions are kept) | `path`/`paths` |
| `list_presets` | The user's develop presets, with their settings | |
| `apply_preset` | Apply a preset (tone, presence, colour, curve, mixer, detail, profile; white balance if the preset has it) | `preset`, `path`/`paths`, `new_version` |
| `save_preset` | Save a photo's look as a preset | `path`, `name`, `white_balance` |
| `set_rating` | Star rating, 0 removes it | `path`/`paths`, `stars` |
| `list_versions` | A photo's saved versions | `path` |
| `save_version` | Save the photo's current edit as a version (named `AI · <name>`) | `path`, `name` (optional) |
| `restore_version` | Make a saved version the current edit (itself saved as an AI version) | `path`, `name` |
| `check_photo` | Render the photo with its edit and measure it: histogram, clipping, contrast, colour cast, colours, sharpness, plus plain-language notes | `path`, `long_edge` (256-2000, default 800) |
| `check_photos` | Measure many photos at once and return one short row per photo: when it was taken, rating, edited, sharpness (`detail`), mean brightness, clipping and the colour cast in neutral areas; with `focus: true`, also the full-size focus measurement. Up to `count` photos per call; call again with `next_start` until it is null | `folder` or `paths`, `start` (default 0), `count` (1-60, default 30), `long_edge` (256-2000, default 512), `focus` (default false) |
| `cull_photos` | Rate a whole folder by focus: group the photos into bursts by capture time and suggest 4 stars for each burst's sharpest frame, 3 for frames nearly as sharp and 1 for the rest. Returns one short row per burst, with those worth a look marked; `apply: true` saves the ratings. A large folder takes several calls while it measures (status `measuring`); measurements are kept | `folder`, `apply` (default false), `replace_ratings` (default false), `gap_seconds` (default 60), `sharp_px` (3.5), `near_px` (0.5), `soft_px` (5.5) |
| `auto_white_balance` | Let Develop set the white balance itself, as the **Auto** button does: from the neutral areas when enough of the picture is grey or white, else from skin when there are faces (firelight, tungsten); temperature (and tint) only, never exposure. Saved in the photo's AI version. Returns per photo the temperature, the method, readings before and after, and a `check` note when worth a look. About a second per photo, four at a time | `paths` (or `path`) |
| `check_focus` | Measure focus on the full-size photo and return a 100% crop (512 x 512 px) of its sharpest area | `path`, `include_crop` (default true) |
| `render_preview` | Render the photo with its edit and return the picture | `path`, `long_edge` (256-3000, default 1024) |
| `export` | Write JPEG, PNG or TIFF files with each photo's edit | `path`/`paths`, `folder`, `format`, `long_edge`, `quality` |
| `convert_to_dng` | Lossless DNG copies of raw files | `path`/`paths`, `folder` (default: beside the original), `embed_original` |

### Examples

Change two sliders on three photos:

```json
{"name": "set_settings", "arguments": {
  "paths": ["D:\\Shoot\\DSC0101.ARW", "D:\\Shoot\\DSC0102.ARW", "D:\\Shoot\\DSC0103.ARW"],
  "settings": {"exposure": 0.4, "shadows": 35}
}}
```

→ `{"updated": 3, ...}`; each photo's result is in its version for this request, named like
`AI · Claude: exposure, shadows · 17 Nov 14:32 UTC`.

A black & white version with a darker sky:

```json
{"name": "set_settings", "arguments": {
  "path": "D:\\Shoot\\DSC0101.ARW",
  "settings": {"black_and_white": true, "contrast": 20, "mono_mix": {"blue": -50, "orange": 30}},
  "new_version": true
}}
```

Export the four- and five-star photos for the web:

```json
{"name": "list_photos", "arguments": {"folder": "D:\\Shoot", "min_rating": 4}}
{"name": "export", "arguments": {"paths": ["..."], "folder": "D:\\Shoot\\web", "format": "jpeg", "long_edge": 2048, "quality": 85}}
```

→ `{"exported": ["D:\\Shoot\\web\\DSC0101.jpg", ...], "failed": []}`

A mistake comes back as an error the assistant can read and fix, for example:

```
Unknown setting brightness. Known: temperature, tint, exposure, contrast, ...
```

## Settings reference

`get_settings` returns, and `set_settings` accepts, these names. Values outside a range are
brought inside it.

| Setting | Range | Notes |
|---|---|---|
| `temperature` | 2000-12000 K, or 0 | 0 means the camera's own white balance (as shot). Higher is warmer, as in Lightroom. The value is absolute; `check_photo` reports the photo's own `as_shot_temperature` and the `temperature_now` in use, so a change can be made relative to them (JPEGs start at about 6500 K). As a first step, each level of `blue_minus_red_half` takes about 150 K near 6500 K, 110 K near 5500 K and 35 K near 3000 K, and each level of `green_minus_magenta` about 3 of `tint`; then measure again |
| `tint` | -150 to 150 | Relative to as shot: negative greener, positive more magenta |
| `exposure` | -5 to 5 | Stops |
| `contrast`, `highlights`, `shadows`, `whites`, `blacks` | -100 to 100 | Lightroom's scales |
| `texture`, `clarity`, `dehaze` | -100 to 100 | |
| `vibrance`, `saturation` | -100 to 100 | No effect in black & white |
| `sharpening` | 0 to 150 | Default 40 |
| `sharpen_radius` | 0.5 to 3 | Pixels, default 1 |
| `sharpen_detail`, `sharpen_masking` | 0 to 100 | Defaults 25 and 0 |
| `profile` | name | A camera profile name as shown in Develop, such as `Adobe Color (approx.)` or `Standard (built-in)` |
| `curve` | list of `[x, y]` | Tone curve points, 0 to 1, e.g. `[[0,0],[0.5,0.56],[1,1]]` lifts the midtones |
| `curve_highlights`, `curve_lights`, `curve_darks`, `curve_shadows` | -100 to 100 | The tone curve's regions |
| `hsl_hue`, `hsl_saturation`, `hsl_luminance` | -100 to 100 per colour | Objects keyed by `red`, `orange`, `yellow`, `green`, `aqua`, `blue`, `purple`, `magenta` |
| `black_and_white` | true / false | |
| `mono_mix` | -100 to 100 per colour | How bright each colour becomes in black & white (same keys as above) |
| `crop` | `[left, top, right, bottom]` | 0 to 1 on the photo as stored |
| `straighten` | -45 to 45 | Degrees |
| `lens_correction` | true / false | Lens profile correction |

`get_settings` also reports `retouch_spots` (how many heal and clone spots the photo has) and
`ai_denoise`. Those, and retouching itself, are changed in the app.

## Checking an edit with numbers

`check_photo` lets any assistant judge an edit without seeing it, including text-only and small
local models. It renders the photo with its current edit (losslessly, so the counts are exact)
and returns, on the 0-255 levels of the finished picture:

| Field | What it tells |
|---|---|
| `brightness` | Mean, median and percentiles (p1, p5, p25, p75, p95, p99) of brightness |
| `channels` | The same for red, green and blue |
| `histogram_32_bins_percent` | Brightness histogram: 32 bins of 8 levels each, as % of the picture |
| `clipping_percent` | Pure white (all channels at maximum), any channel at maximum, per channel, pure black |
| `zones_percent` | Share of deep shadows, shadows, midtones, highlights and the brightest tones |
| `contrast` | Standard deviation of brightness, and the p5 to p95 range |
| `skin` | Skin tone of the faces in the photo (found with a small face detector), measured as if the faces were at 20% brightness (Develop shifts exposure for the measurement only; the edit is not changed): `saturation` (about 0.36 looks natural; higher is too warm), `off_skin_line_deg` (hue against the skin line on Develop's vectorscope; below about -6 is too pink or cool), `faces`, and a plain `reading`. For white balance where nothing in the photo is grey or white, such as firelight and tungsten. No face, no reading |
| `white_balance` | The photo's own `as_shot_temperature` and the `temperature_now` in use |
| `neutral_areas` | Colour cast measured on low-colour areas: `blue_minus_red_half` (too cool if positive) and `green_minus_magenta` (too green if positive); within about ±2.5 is neutral |
| `whole_picture_balance` | The same over all mid-tones, a rough backup when there are no grey areas |
| `saturation` | Average saturation, and how much is very strongly coloured |
| `colours` | For each Color Mixer colour: share of the picture, average saturation and brightness |
| `brightness_grid_3x3` | Average brightness of nine areas (rows top to bottom): sky versus ground, dark corners |
| `detail` | Average local contrast: compare photos at the same size for focus, or before and after sharpening |
| `settings_summary` | The main settings of the edit, in one line |
| `notes` | The findings in plain words, with the slider to move |

For example, for a sunset photo pushed one and a half stops too bright with a far too cool
white balance, the notes read:

```
20.54% of the picture is pure white (blown out). Unless that is the sun, a lamp or a
reflection, lower Highlights or Whites.
14.06% more has one colour channel at maximum (bright colours clipping). Lower Highlights,
or Saturation / Vibrance if colours are strong.
Too few neutral areas to measure the white balance directly, but the whole picture leans
blue by 35 levels on average: probably too cool, raise Temperature.
```

Rough guides (every photo is different, and the photographer's intent wins):

- **Clipping:** pure white above about 0.5% usually means lost highlight detail, unless it is the
  sun, a light or a reflection. Pure black above about 1% means lost shadow detail, unless the
  look is meant to be dark.
- **Brightness:** an ordinary daylight scene averages around 100-140; night, low-key and high-key
  photos sit well outside that on purpose.
- **Contrast:** a p5 to p95 range under about 120 looks flat; over about 235 looks harsh.
- **White balance:** neutral areas within ±2.5 levels look neutral. Sunsets and candlelight are
  meant to be warm.

A good loop: `check_photo` before an edit, change one or two settings with `set_settings`,
`check_photo` again, compare.

### Many photos: check_photos

A full `check_photo` report is a few thousand characters. For a whole folder (culling for focus,
finding colour casts across a shoot), use `check_photos` instead: it measures up to 60 photos per
call (30 by default, four at a time) and returns one row each, in a fraction of the space, so a
model with a small context does not run out of room. A call stops starting new measurements after
about 35 seconds, so it returns before most assistants' tool timeout, and may return fewer photos
than `count`. Each response has `columns`, `rows`, `total` and `next_start`; call again with
`start` set to `next_start` until it is `null`. Each row includes `taken`, when the photo was
taken by the camera's clock, so an assistant can tell a burst (frames seconds apart) from
separate shots of a similar scene.
`detail` is only comparable between photos of the same scene rendered at the same size, so compare
neighbouring frames. Then use `check_photo` on the few photos you are going to edit.

### Focus: check_focus, and check_photos with focus

`detail` is measured on a small render and averaged over the whole picture, so it cannot tell a
slightly missed focus or a sharp subject on a soft background. For focus, Develop measures the
**full-size** picture instead: it finds edges that rise clearly above the photo's noise, measures
how many pixels each takes to go from dark to light, and reports the sharpest area of the photo.

| Field | What it tells |
|---|---|
| `sharpest_edge_px` | Edge width in full-size pixels in the sharpest area. About 2.7 is the sharpest this can read; crisp detail usually reads 2.5-3.5; bigger is softer |
| `sharpest_edge_across_px`, `sharpest_edge_down_px` | The same, for edges crossed left to right and top to bottom. A large difference can mean motion blur or camera shake in the softer direction (or few edges that way in that area) |
| `sharpest_at` | Where the sharpest area is, as fractions of the width and height (0, 0 = top left) |
| `median_edge_px` | Typical of the whole picture: high with a shallow depth of field |
| `verdict` | sharp (up to 3.5), slightly soft (up to 5.5) or soft |

`check_focus` also returns the sharpest area as a 100% crop, 512 x 512 px and not resized, so an
assistant that can see images can confirm that it is the subject (an eye, not the background) and
that it looks sharp. `check_photos` with `focus: true` adds `focus_sharpest_edge_px`,
`focus_median_edge_px` and `focus_sharpest_at` to every row; it is slower (about 1-2 seconds per
photo), so use it when culling for focus.

The numbers are a guide, best for comparing frames of the same scene (a burst, neighbouring
shots): a frame clearly softer than its neighbours missed focus or moved. They are measured on the
photo as edited, so sharpening, texture and clarity make edges steeper, and very noisy photos read
less reliably.

### Culling a folder: cull_photos

Rating a shoot by focus is mostly arithmetic, so Develop does it and the assistant only reviews.
`cull_photos` measures every photo's focus as above (about a second or two per photo; a call stops
after about 35 seconds with `status: "measuring"`, so a large folder takes several calls with the
same arguments). The measurements are kept in the settings folder until the photo or its edit
changes, so calling again, even in a new session, measures nothing twice.

When everything is measured (`status: "done"`), it groups the photos into **bursts**: frames taken
within `gap_seconds` (60) of the one before. In each burst:

| Stars | Which frames |
|---|---|
| 4 | The sharpest frame, if its edge width is at most `soft_px` (5.5) |
| 3 | Frames at most `sharp_px` (3.5) wide and within `near_px` (0.5) of the sharpest |
| 1 | Everything else, including whole bursts where even the sharpest frame is soft |

It returns one row per burst (first and last file, number of frames, when, the keeper and its edge
width, the 3-star frames) and a `check` note on bursts worth a look: a keeper that is only
slightly sharp, a sharpest area near the edge of the frame (perhaps the background, not the
subject), two frames almost equally sharp, or a soft burst close to the limit. An assistant that
can see images can open those keepers with `check_focus`. Nothing is rated until it is called with
`apply: true`; photos that already have a rating keep it unless `replace_ratings: true`.

A 400-photo folder takes about 10-15 minutes to measure (longer while a local AI model is using the
same graphics card) and then rates in one call, instead of the assistant reading and rating 20 rows
at a time.

## Good practice

For people and for assistants:

1. **Versions are automatic.** Each request's changes to a photo are saved as one `AI · ...`
   version and your edit is kept as `Before AI · ...` (see [AI versions](#ai-versions)). To go
   back, open the version you want in Develop, or ask the assistant to `restore_version` it.
2. **Look (or measure) before you batch.** Change one photo, check it with `render_preview` or
   `check_photo`, and only then apply the same settings (or a preset) to the rest.
3. **Change a few things at a time,** in Develop's order: white balance, exposure, contrast and the
   tone sliders, presence, then colour. Small steps are easier to judge.
4. **Use presets for consistency.** Save the finished look of one photo with `save_preset`, then
   `apply_preset` to the series. Presets leave crop, rotation and retouching alone.
5. **Match a series by white balance first.** Give every photo shot in the same light the same
   `temperature` and `tint`, then fix exposure photo by photo.
6. **Try things on a copy.** For a big experiment, copy the folder (with its `.dvlp` files) and point
   the assistant at the copy.
7. **Watch in Develop if you like.** With Develop open on the same folder, the assistant's ratings,
   versions and edits appear as they are saved, and the open photo updates (Undo goes back). The
   newest change wins, so avoid editing the same photo at the same moment as the assistant.
8. **Keep previews small.** 1024 pixels is plenty to judge a look and keeps conversations fast;
   use larger previews only to check fine detail.
9. **Ask before exporting or resetting many photos.** Exports write files; a reset clears the edit
   (versions survive).

## Privacy and safety

- **What the assistant can see:** the folders and files you point it at, their names and paths,
  ratings and edit settings. `render_preview` sends a picture of the photo to the assistant. With a
  cloud AI (Claude, ChatGPT, Gemini and others) that picture leaves your computer, as anything you
  paste into the chat would. With a local model (LM Studio and similar) nothing leaves it.
- **What it can change:** edits (`.dvlp` files), ratings, versions and presets. It can create new
  files with `export` and `convert_to_dng`. It cannot delete photos, and it never changes the
  original raw files.
- **No network access by default.** `--mcp` talks only to the app that started it. `--mcp-http`
  listens on this computer only and refuses requests without the token.
- **Undo:** Develop's Undo does not cover changes made through MCP while the photo was closed.
  Every such change is a labelled version instead, and your own edit is kept as `Before AI · ...`.
- **Your versions are protected:** no tool can change or delete them.

## Troubleshooting

- **The tools do not show up.** Check the path to `develop.exe` (open it in Explorer), the doubled
  backslashes in JSON, and restart the AI app after editing its configuration.
- **"Rendering ... failed".** `render_preview` and `export` run Develop to render; make sure the
  photo opens in Develop itself. A window may flash briefly while a render runs.
- **Slow exports time out.** Export fewer photos per call, or raise the app's tool timeout (see
  Gemini CLI above; other apps have similar settings).
- **Changes do not appear in Develop.** They should appear within a second in the open folder. If they
  do not (some network shares do not report changes promptly; Develop then checks every few
  seconds), wait a moment or open the folder again. Ratings and edits made through MCP are already
  saved in the `.dvlp` files.
- **`--mcp-http` prints nothing on Windows.** Run it from a terminal (Windows Terminal, PowerShell).
  The address and token are also in `mcp-http.txt` in the settings folder.
- **401 Unauthorized.** The `Authorization: Bearer <token>` header is missing or the token is wrong.

## Protocol details

For developers connecting their own client:

- **Protocol version** `2025-06-18` (the client's version is echoed back when it sends one).
- **Capabilities:** tools only (no resources, prompts or sampling). `initialize` returns short
  instructions for the assistant.
- **stdio:** newline-delimited JSON-RPC 2.0 on standard input and output.
- **HTTP:** `POST /mcp` with a JSON-RPC message (or batch) in the body; the reply is
  `application/json`. Notifications get `202 Accepted`. There is no event stream and no session:
  every request stands alone. `GET` returns `405`.
- **Errors:** protocol errors use JSON-RPC error codes; a tool that fails returns a normal result
  with `isError: true` and a readable message.

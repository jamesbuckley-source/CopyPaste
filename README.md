# CopyPaste

A clipboard history manager for Windows: Tauri 2 + React 19 + TypeScript + Tailwind CSS 4.

Everything stays on your machine. Clip contents are stored locally in SQLite as plain text.

<p align="center">
  <a href="https://github.com/jamesbuckley-source/CopyPaste/releases/latest/download/CopyPaste_x64-setup.exe">
    <img src="https://img.shields.io/badge/Download_CopyPaste_for_Windows-0.13.2-6d8cff?style=for-the-badge&logo=windows11&logoColor=white" alt="Download CopyPaste for Windows">
  </a>
</p>

<p align="center"><sub>Per-user NSIS installer · no admin prompt · Windows 10 and 11</sub></p>

## Download

**[Download CopyPaste for Windows](https://github.com/jamesbuckley-source/CopyPaste/releases/latest/download/CopyPaste_x64-setup.exe)**
— a per-user NSIS installer, no elevation prompt, roughly 5 MB.

All versions, notes and artefacts are on the
[releases page](https://github.com/jamesbuckley-source/CopyPaste/releases/latest). Installed copies
update themselves: *Settings → Updates* checks GitHub once a day and installs signed releases in
place.

## Screenshots

![CopyPaste main window](docs/screenshots/main-interface.png)

*The history view: search and filters on the left, clips in the middle, and the preview pane with
the colour scheme, notes, collections, archives and tags on the right. The **Add clip** button sits
in the toolbar, and the version number at the bottom right is also the manual update-check button.*

![Quick preview of an image clip](docs/screenshots/quick-preview.png)

*`Space` opens the selected clip over a dimmed history. Images get zoom controls at the bottom; text
clips open in a larger editor.*

![First-run setup wizard](docs/screenshots/first-run-wizard.png)

*The first-run wizard covers the summon hotkey, paste and privacy choices, secret handling,
retention, theme and sounds. It can be re-run from Settings at any time.*

## What it does

- **Captures** text, images and copied file lists as you copy them.
- **Drops in** files: dragging files from Explorer onto the window creates an image clip, a text clip or a file-list clip.
- **Keeps GIFs animated** — copying an image from a browser recovers the source GIF and copies it back as a file, so every frame survives; the recovered animation is also what the app previews.
- **Recognises color codes** — a copied hex or `rgb()` value becomes a swatch with the code laid
  over it, with its own sidebar entry. Screenshot clips read their own colour scheme automatically,
  keeping the accents rather than a wall of greys.
- **Summons** with a global hotkey (`Alt+V` by default) as a frameless overlay, or from the tray.
- **Searches** as you type, ranked by exact match, word boundaries, position and recency. The
  Text view expands into Link, Email, Path, JSON and Secrets sub-views, and the state is remembered.
- **De-duplicates**: copying the same thing twice promotes the existing entry instead of
  stacking duplicates, and the clip you reuse jumps back to the top.
- **Organises** with pinning, collections and tags; collections can be turned into **archives**
  that stay out of the history and survive every bulk delete. Collections, archives and tags each
  have a filter box and a compact “Show all” list, and each sidebar section folds away with its
  state remembered.
- **Previews** the full text, the image, or each file path before you commit to using it. `Space`
  opens a larger quick preview with zoom for images and an auto-saving editor for text.
- **Writes clips by hand**: the **Add clip** button in the toolbar opens a small window with
  Markdown formatting buttons, and text containing Markdown is stored with a **Markdown** badge.
- **Two layouts** — rows for reading text, cards for browsing images — with a draggable card-size
  slider and a separate interface scale.
- **Themes** with dark and light presets, or a custom palette picked color by color.
- **Reads text out of screenshots** with the Windows OCR engine and makes it searchable; a hover
  button copies that text instead of the image.
- **Orders the history** by last use or capture date, in either direction, and remembers a
  list-or-cards layout per category.
- **Reveals** clips in Explorer from a right-click menu; each archive owns a folder there.
- **Updates itself** from signed GitHub releases, with a daily check you can turn off.
- **Pastes back** on demand into whatever you were working in (opt-in; off by default).
- **Pauses capture** on demand — a private mode for 15 minutes, an hour, or until restart.
- **Finds things with operators** — `kind:image`, `tag:invoice`, `app:code`, `before:2024-05-01`,
  `color:#ff8800`, `in:archive` — and saves any query to the sidebar as a smart view.
- **Acts on what it recognises**: links open in the browser, paths open or reveal in Explorer,
  JSON can be pretty-printed, colors come with their hex and `rgb()` values. Recognised rows also
  carry their action as an inline button, so *Open link* / *Open Folder* is one click away.
- **Saves and drags** clips out: *Save as…* writes the image, the files or the text to a folder,
  and rows can be dragged straight into Explorer, an editor or a chat window.
- **Selects many at once** and pins, tags, files or deletes the whole selection.
- **Exports in bulk**: *Save to…* in the selection bar writes every picked clip into one folder.
- **Exports Markdown notes**: with a vault folder set in *Settings → Markdown*, a text clip
  becomes a dated note with YAML front matter — from the right-click menu, the preview pane,
  the selection bar or `Ctrl+Shift+M`.
- **Searches emoji** from a Utilities section, with favourites that survive restarts.
- **Browses GIPHY** from the same Utilities section, starring GIFs and saving image clips to a
  personal GIF shelf; click any result to copy the animation. The shelf also imports GIF files
  from disk.
- **Annotates** clips with a searchable note, and edits a text clip in place from the preview pane.
- **Samples colors** from anywhere on screen with the eyedropper in the title bar; a magnifier
  beside the crosshair previews the pixel and its hex/RGB values before you click.

## Requirements

| | |
|---|---|
| OS | Windows 10/11 |
| Node | 20+ |
| Rust | 1.85+ (`rustup`; the backend uses edition 2024) |
| Build tools | MSVC C++ build tools + Windows SDK |
| WebView2 | Preinstalled on Windows 11; a runtime download on older Windows 10 |

The Rust backend compiles SQLite from source, so there is no external `sqlite3.dll` to install.

## Getting started

```powershell
npm install
npm run tauri dev          # dev build with hot reload
npm run tauri build        # optimised build + NSIS installer
```

Dev mode serves the UI on **port 5173** and HMR on 5174. (Tauri's scaffold default of 1420 falls
inside a Windows reserved port range on machines with Hyper-V or WSL enabled, where it fails with
`EACCES`.)

The app starts hidden. Press **Alt+V** or click the tray icon to bring it up. The very first
launch shows the window instead, so it does not look like nothing happened.

## Building an installer

`npm run tauri build` produces a per-user NSIS installer:

```
src-tauri/target/release/bundle/nsis/CopyPaste_<version>_x64-setup.exe
```

Releases published from a `v*` tag also attach the same installer under a stable name,
[`CopyPaste_x64-setup.exe`](https://github.com/jamesbuckley-source/CopyPaste/releases/latest/download/CopyPaste_x64-setup.exe),
which is what the download link above uses. The tag build is signed with the updater key so
installed copies can update in place.

It installs to `%LOCALAPPDATA%\CopyPaste` with no elevation prompt and registers an entry in
*Apps & features* for uninstalling. NSIS is the open-source installer generator Tauri uses —
no telemetry, no bundled extras. Your data lives separately in
`%APPDATA%\com.copypaste.app`, so uninstalling and reinstalling keeps your history (delete
that folder to start clean).

## Tray and background behaviour

Both, by design:

- **Tray icon** for the lifetime of the process. Left click toggles the overlay; right click
  opens *Open CopyPaste*, *Clear unpinned history*, *Settings…* and *Quit*.
- **Taskbar button** by default, so the window behaves like a normal app. Turn off
  *Appearance → Show in the taskbar* to keep it tray-only. The frameless title bar carries its
  own minimize / maximize / close buttons and can be dragged anywhere along its length.
- **Closing does not quit.** The ✕ button *and* Alt+F4 hide the window, leaving the clipboard
  watcher running so history keeps accumulating. Quitting is deliberately only available from
  the tray menu.
- **Losing focus keeps the window open.** It is an inspection surface, so it should not vanish
  when you click away to look something up. *Settings → Window → Hide when focus is lost* opts
  back into the overlay behaviour.
- **Not pinned on top by default.** The overlay behaves like an ordinary window unless
  *Settings → Window → Always on top* is enabled, which keeps it above whatever you are working in.
- **One instance at a time.** Launching again from the Start Menu raises the running instance
  rather than starting a second copy, which would otherwise add a duplicate tray icon and fail
  to claim the global hotkey a second time.
- **Launch at login** is a setting; when enabled the app registers itself with the Windows
  startup list and returns after a reboot, still hidden.

## Keyboard

| Key | Action |
|---|---|
| `Alt+V` | Show / hide the overlay (configurable) |
| `Ctrl+Alt+P` | Toggle private mode, pausing capture (configurable) |
| `↑` `↓` | Move the selection |
| `PgUp` `PgDn` `Home` `End` | Jump through the list |
| `Enter` | Copy the selected clip, and paste it if auto-paste is on |
| `Space` | Open the quick preview of the selected clip; `Space` or `Esc` closes it |
| `Ctrl+1` … `Ctrl+9` | Copy the clip with that slot number, and paste it if auto-paste is on |
| Double click | Copy the clip, and paste it if auto-paste is on |
| `Ctrl+Click` / `Shift+Click` | Add a clip to the selection, or select a range |
| `Ctrl+A` | Select every visible clip |
| `Ctrl+P` | Pin / unpin |
| `Ctrl+Shift+M` | Export the selected text clip(s) as Markdown notes, then hide the overlay |
| `Del` | Delete the selected clip, or the whole selection (only while the search box is empty) |
| `Esc` | Clear the selection, the clip or settings search, close settings, or hide the window |

## Searching

The search box ranks as you type, and it also understands operators. Anything it does not
recognise stays part of the text query, so a stray colon never makes results vanish.

| Operator | Example | Effect |
|---|---|---|
| `kind:` | `kind:image` | Restrict to text, image or file clips |
| `type:` | `type:url` | Restrict to a recognised payload: `url`, `email`, `path`, `json`, `color`, `markdown`, `secret` |
| `tag:` | `tag:invoice` | Clips carrying that tag |
| `collection:` | `collection:Work` | Clips filed in that collection |
| `app:` | `app:chrome` | Substring match on the source application |
| `before:` | `before:2026-05-01` | Captured before that date |
| `color:` | `color:#ff8800` | Exact color match |
| `pinned:` | `pinned:yes` | Pinned clips only |
| `in:archive` | `kind:image in:archive` | Also reach clips kept in an archive |

Each operator appears as a chip under the input; click a chip to drop it. The **Archives** button
next to the sort dropdown does the same thing without typing.

**Save** turns the current query into a sidebar entry under *Saved searches*. Saved searches are
stored with the rest of the settings, re-run with one click, and can be renamed or deleted from
their right-click menu.

## Private mode

*Pause capture* stops recording without changing any settings:

- From the tray menu: 15 minutes, 1 hour, until restart, or resume.
- From the keyboard: `Ctrl+Alt+P` toggles a 15-minute pause.
- From *Settings → Activation*, which also shows the current state.

While paused the watcher keeps advancing its clipboard counter, so whatever was copied during the
pause is never recorded when capture resumes. The status bar shows a pause chip with the remaining
time — click it to resume.

## Secrets

CopyPaste flags text that looks like a credential — private-key blocks, `Authorization` headers,
URLs with embedded passwords, JWTs and the usual token prefixes (`ghp_`, `sk_live_`, `xox*`, `AIza`,
`AKIA`, …). The flag is decided at capture time on the plaintext and only the confidence is kept, so
the history list can warn without decrypting anything. A flagged clip's preview is blurred until you
press *Reveal*, and the preview pane says how sure the match is. In card view the reveal control is a
shield icon in the top-left corner of the thumbnail instead of sitting at the end of the preview
line, so it never lands under the hover actions. Flagged text clips carry a gold
**Secret** pill instead of their usual Text/Link label, so they stand out in both layouts.

Detection is only a guess, so any text clip can be marked by hand: the shield button in the
preview pane's action row turns the flag on, and pressing it again clears it and leaves an ordinary
text clip. A manual mark is stored at the highest confidence, so it blurs and asks the same way a
detected one does — including before a Markdown export writes it to disk.

*Settings → Capture* adds two options: *Guess from context* (keyword plus entropy matching, off by
default because it is noisier) and *Leave secrets out of history*, which skips a flagged copy instead
of storing it. Detection runs locally and match details are never logged.

## Sounds

*Settings → Sounds* plays a short tick when a copy succeeds and a lower tone when one fails. Both
ship with the app; each can be switched off, and either can be replaced with your own `.wav`.

## Bulk actions

`Ctrl+Click` extends the selection — the clip that was already highlighted joins the group on the
first such click — `Shift+Click` selects a range and `Ctrl+A` takes everything visible. A bar above
the list then offers **Pin**, **Unpin**, **Tag…**, **Collect…**, **Save to…** and **Delete**;
`Del` deletes the whole selection, and `Esc` clears it. The right-click menu gains *Export N as
Markdown* for the same selection, which writes one note per text clip and counts the ones it had to
skip.

*Collect…* accepts an existing collection name or a new one — typing a name that does not exist
creates the collection and files the selection into it. *Save to…* exports every picked clip into
one folder, the same way *Save as…* handles a single clip. The bar wraps rather than squeezing its
buttons, so every action keeps its full label in a narrow window.

Right-clicking one of the selected clips opens the same actions for the whole selection — *Pin 3
clips*, *Tag 3 clips…*, *Delete 3 clips* — above the usual single-clip entries, and right-clicking
never disturbs what is selected. Right-clicking an unselected clip acts on that clip alone.

The quick-paste number for each of the first nine clips is drawn over the bottom-right corner of
its thumbnail in both layouts, so it never pushes the preview text along.

## When something goes wrong

Failures and confirmations float as a card just under the title bar instead of being inserted into
the layout, so nothing shifts when one appears — with **Copy** and **Dismiss**. The text is
selectable too, so a message can be pasted into an issue instead of retyped. The banner reports the
first problem only; dismissing it lets the next one through.

## Content, saving and dragging

Text that is entirely a link, an email address, a Windows path or JSON is tagged with that shape in
the list and gains matching actions in the preview pane and the right-click menu: *Open link*,
*Send email*, *Open*, *Show in Explorer*, and *Format JSON* (a view-only pretty-print — the stored
payload is never rewritten).

*Save as…* writes the clip into a folder you pick: the stored PNG for an image, a copy of each
original file for a file list, and a `.txt` (plus an `.html` copy when one was captured) for text.
Name collisions get a ` (2)` suffix instead of overwriting.

**Markdown notes.** *Export as Markdown note…* writes a text clip into the vault folder chosen in
*Settings → Markdown*. The filename is `YYYY-MM-DD <first line>.md` (leading Markdown markers are
dropped and characters Windows cannot name are replaced), the body is copied as-is with line
endings normalised to LF, and YAML front matter — `created`, `source`, `tags` and the clip id — is
on by default. The date prefix and the front matter can each be turned off. A selection exports one
note per text clip and skips images and file lists. The folder must already exist, so a vault that
moved reports the problem instead of gaining an empty folder, and a clip flagged as a likely
credential asks before its plain text is written to disk.

Rows and cards can be dragged out of the overlay. Text travels as `text/plain`, images and file
lists as `text/uri-list`, and the payload is pre-loaded when you hover a row so the drop target
receives it immediately. Dragging a clip onto a sidebar collection or archive files it there;
dragging a multi-clip selection files all of the selected clips.

Colour swatches extracted from an image also copy on click and are recorded as colour clips, so
they appear in the Colors view and in history rather than only reaching the clipboard.

**Dropping in.** Dragging files from Explorer onto the window shows a drop overlay and creates a
clip on release: a single image becomes an image clip, a single small UTF-8 text file becomes a
text clip, and anything else becomes one file-list clip of every dropped path. Dropped clips
dedupe against existing history exactly like copied ones.

**Rich text.** When a source app offers HTML or RTF next to the plain text, both are stored
(in their own columns) and written back on copy — so pasting into Word or Outlook keeps
the formatting while everything else still gets the plain text. Turn off *Settings → Pasting →
Paste with formatting* to paste plain text only.

**Double-click.** Activating a clip with a double click copies it and dismisses the overlay.
*Settings → Pasting → Close after double-click* can keep the window open instead, which still
copies the clip.

**Auto-clear.** *Settings → Pasting → Clear the clipboard after use* empties the Windows clipboard
after a copy has been used. The delay is a setting; `0` clears as soon as the overlay hides, with a
short floor so it cannot race an auto-paste. If anything else has touched the clipboard since, the
timer leaves it alone rather than destroying something you copied in the meantime.

## Settings

On a fresh install, a three-step **setup wizard** opens the first time the window appears. It
covers the global hotkey, *Start with Windows*, auto-paste, image and file capture,
password-manager exclusions, a retention preset, how likely secrets are handled, and a theme and
sounds. Choices land in the same settings store as the Settings window and can be changed there
later; *Settings → Updates → Setup wizard* reopens the wizard. Upgraded installations are marked
complete by the store migration and are never interrupted by it.

Settings open in their own window — the gear in the title bar or *Settings…* in the tray menu
opens (or focuses) a normal decorated window with minimise, maximise and close buttons, leaving
the overlay a pure history surface. `Esc` closes it.

The search box at the top of the settings page narrows the page to matching settings as you
type. Name matches rank above description matches, and a section whose own heading matches keeps
all of its rows — `Esc` clears the filter before it leaves settings.

Because settings live in their own window, every change is broadcast to the other window as it is
saved, and the main window re-reads them when it regains focus. Switching the theme in Settings
repaints the history window immediately instead of waiting for it to be reopened.

*Storage* holds two long-lived choices besides the housekeeping buttons:

- **Data folder** — *Change…* copies the database and the image files to the chosen folder, then
  restarts into it. The old copy is deleted on the next start. The location is remembered in a
  small `storage.json` pointer inside the default app-data folder, because it has to be known
  before the database can be opened. If the chosen folder is missing at startup (an unplugged
  drive), the app falls back to the default one rather than refusing to start.
- **Maximum age** — retention can be expressed in **hours or days**; `0` keeps everything.
  Archived clips are exempt. Maximum clips and the pinned exemption behave as before.

*Markdown* holds the note-export choices:

| Setting | Default | Notes |
|---|---|---|
| Export folder | **empty** | The vault notes are written into. Empty asks for a folder at export time; *Clear* goes back to asking |
| Front matter | **on** | Writes `created`, `source`, `tags` and the clip id as a quoted YAML block |
| Date prefix | **on** | Names each note `YYYY-MM-DD <first line>.md` |

The folder is never created: if the vault has been moved or its drive unplugged, the export says so
instead of quietly making a new empty folder and looking successful.

## Appearance

*Settings → Appearance* controls:

| Setting | Default | Notes |
|---|---|---|
| Show in the taskbar | **on** | Off makes the app tray-only |
| View per category | Images and colors: cards, others: rows | Each history category remembers its own list/cards choice |
| Card size | 100% | Drag the slider in the toolbar (60–200%); each history category keeps its own |
| Text scale | 100% | Scales the whole interface, 70–200% |
| Sidebar lists | 6 items | Collections and archives show this many before **Show all**; tags stop at 10 |
| Preview pane | shown | Collapse it with the chevron at the pane's top-left |
| Theme | Dark | Dark, Light, or Custom with a color picker per token |

Each sidebar category (All clips, Text, Images, Files, Pinned, and collections/tags together) keeps
its own layout and card size: switching to cards for Images does not drag Text along with it, and
images start in cards because a thumbnail grid is how you actually recognise a screenshot. The Text
row's chevron reveals Link, Email, Path, JSON and Secrets sub-views; double-clicking anywhere on
the row does the same, and the choice is remembered.

Images are also read with the Windows OCR engine in the background; the recognised text is
indexed for search, shown under the preview, and copyable from a button on the card.

Themes are stored as a palette of twelve color tokens. Rather than shipping two stylesheets, the
selected palette is written onto the document as `--color-*` custom properties, which is what the
Tailwind utilities already compile to — so a theme change repaints everything without a rebuild.
Custom mode exposes every token as a swatch plus a hex field.

Interface scaling uses CSS `zoom` on `#root`, which re-lays-out instead of resampling, so text
stays crisp. The layout fills the window at every scale — zoomed elements resolve their
percentages against an already-divided containing block, so nothing else may compensate.

In card view the grid takes the detail pane's width, so more columns fit; switch back to List to
edit tags and collections.

## Colors

When the *entire* thing you copy is a color code — `#rgb`, `#rrggbb`, `#rrggbbaa`, `#rgba`,
`rgb(…)` or `rgba(…)` — the clip is recognised as a color. Prose that merely mentions a code
stays text, so one stray hashtag does not hijack the swatch view.

Color clips render as a filled swatch with the copied code painted on top (the label switches
between black and white based on the sample's luminance), keep their normal text payload for
search and copy, and are listed under **Colors** in the sidebar. The preview pane and every
thumbnail show both the hex and `rgb()` values.

For image clips, the preview pane's **Color scheme** box lists the dominant colors of the
screenshot. It fills in on its own when an image opens — the extraction runs off the UI thread and
is stored with the clip, so reopening a screenshot shows the same scheme instantly instead of
reading the image again. Colors are clustered in OKLab, so "close" means close to the eye, and at
most two of the entries may be near-neutral: a screenshot is mostly white, grey and black, and on
raw frequency those used to fill every slot before an accent was ever reached. Click a swatch to
copy its hex value; the scheme stays on screen while you click through the others.
**Make Color Palette** files the scheme into a new archive named **Color Palette 1**,
**Color Palette 2** and so on, so it can be kept as a set.

The same whole-payload test classifies links, email addresses, Windows paths and JSON. Those clips
are labelled **Link**, **Email**, **Path** or **JSON** in the list and gain the actions that fit
them — *Open link*, *Open*, *Show in Explorer*, *Format JSON* — plus a `type:` search operator.

Hex values are recognised with or without a leading `#` — `#ff8800`, `ff8800`, `0xFF8800` and the
short `f80` all work — and `rgb()` accepts the CSS Color 4 forms (`rgb(255 136 0)`,
`rgb(255 136 0 / 50%)`). A bare value has to contain both a digit and a letter, which keeps words
like `facade` and numbers like `123456` out of the color view. Hex codes are shown upper-cased
(`#FF8800`) wherever the app displays them; the stored payload keeps whatever case was copied, so
copying it back out is byte-for-byte what you had.

## The preview pane

The pane is available in both layouts, and its left edge is a drag handle: the width is clamped to
a readable 200 px minimum and 80% of the window, and the chosen width is remembered. Collapsed to a
rail, the whole strip is a button that brings the pane back. Narrowing the pane drops the word
*Copy* from the primary button — every action keeps its icon rather than being squeezed out.

Clips whose content has an obvious next step carry that step as a button, in the pane and in the
row or card itself: a link gets **Open link**, a path gets **Open Folder** (revealed in Explorer) and
a folder icon instead of the plain text glyph.

### Quick preview

`Space` with one clip selected opens a larger view over the list, with the history dimmed and
blurred behind it. Images fill the overlay with zoom controls (50–400%) centred at the bottom, and a
reset that returns to 100%; the mouse wheel zooms too, one step per burst so a trackpad does not fly
through the range. Zoom changes animate quickly, and are instant when Windows is set to reduce
motion. A zoomed image is moved by dragging it around rather than with scrollbars, and the drag is
clamped so the image can never be lost off the edge. Text clips open in a larger monospace panel
with an **Edit** button and a lighter editing surface, so the field reads as a field: the edits
save themselves a moment after typing stops and again when you close, so the overlay never holds
unsaved input. A Markdown clip also gets the same formatting bar as the Add clip window while it is
being edited. A clip flagged as a possible secret stays blurred until you reveal it, and cannot be
edited while it is hidden. `Space` or `Esc` closes the overlay.

Clicking a clip moves the keyboard into the list, so `Space`, the arrow keys and `Del` act on the
selection rather than on the search box — which is focused whenever the window appears. Typing a
query still goes to the search box as before.

## Collections, archives and tags

Hovering a collection in the sidebar reveals two actions: archive it, or delete it.

| | Collections | Archive |
|---|---|---|
| How many per clip | As many as you like | Exactly one, or none |
| Where they are edited | The **Add to Collections** picker in the preview pane | The Archive dropdown, under it |
| Effect on the history | None | Hides the clip from the history views and protects it from Clear history and retention |

The backend enforces the split: `set_clip_collections` only rewrites ordinary memberships, so
ticking a collection can never quietly unarchive a clip, and `set_clip_archive` clears any previous
archive before setting the new one.

The **Add to Collections** picker filters as you type, shows the configured number of entries with
**Show all** for the rest, and keeps the clip's own collections as removable pills above the list —
so the checklist only ever offers collections the clip is not in yet.

- An **archive** keeps its clips out of All clips, Text, Images, Files, Pinned, tags and search
  results; selecting the archive itself is how you reach them. The search bar's **Archives**
  toggle (or `in:archive`) reaches them from anywhere.
- Archived clips survive both *Clear history* variants and the retention limits, so an archive is
  a safe place for the things history should never eat.
- Unarchiving the collection puts its clips straight back into the history views.
- **New archive** sits under the Archives section and creates the collection already archived, so
  an archive can be made without first making a collection and then converting it.
- Deleting a collection asks first, and only the grouping goes away — the clips never do.
- Every archive owns a folder under `archives/` in the data directory. Right-click the archive in
  the sidebar to open it; renaming the collection renames the folder, and unarchiving removes it
  once it is empty.

Hovering a list row or a card also offers a **copy** button. It puts the clip on the clipboard and
leaves the window open, unlike `Enter` or a double click, which copy-and-dismiss.

The archive picker in the preview pane is a dropdown of every archive plus *Not archived*, so
filing a clip takes one click instead of scanning a radio list.

Right-clicking a clip opens a small menu with an icon on every entry. *Copy* leads the list, and
*Add to Collection* / *Add to Archive* open fly-outs of the matching collections; images and file
lists can also be revealed in Explorer (the stored PNG, or the original path with a fallback to
its folder), and *Pin* works from there too.

Tags are managed from the sidebar: right-click one to rename it, give it a color dot, or delete
it. Renaming onto a name that already exists merges the two tags rather than failing, and the
search index is rebuilt so results follow the new name straight away. The **Tags** section also has
a filter box: type to narrow the tag list, press `Enter` to open the first match, `Esc` to clear it.
Collections and Archives use the same filter box, show the configured number of entries, and offer
**Show all** with a compact scrolling list when there are more.

## Utilities: emoji

*Utilities → Emoji* opens a searchable library of emoji, one category at a time (the chips under
the search box switch sections, and *All* shows everything). The search matches names and keywords
("rocket", "thank you", "deploy") across every category, clicking an emoji puts it on the
clipboard, and right-clicking pins it to **Favourites** at the top or removes it from Favourites
while it stays in the library. Favourites are stored with the rest of the settings. The search box
has a clear button, and the preview-size slider above the grid resizes cells just like the GIF
utility; the size is remembered between sessions.

The whole library draws from bundled [Twemoji](https://github.com/jdecked/twemoji) artwork (v17.0.3,
by jdecked and other contributors, [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) — see
`public/emoji/twemoji/NOTICE.txt`), so cells no longer depend on how Windows rasterises colour
glyphs. The clipboard still receives the Unicode character, not an image. If an asset is ever
missing, the cell falls back to the font glyph.

The grid is virtualised and opens on a single category, so the first paint builds about a hundred
buttons instead of the whole library. Row positions are precomputed for a binary-search scroll
window and scroll events are coalesced to one render per frame.

Regional-indicator flags are left out on purpose: Windows has no flag glyphs, so they would render
as bare letter pairs.

## Utilities: GIFs

*Utilities → GIFs* searches GIPHY and copies the animation you pick: the downloaded GIF is placed
on the clipboard as a file (image targets paste the animation, text targets get its address).
Search needs a free GIPHY API key, entered once in *Settings → Utilities*; it is kept in the local
settings and only used for these requests.

Star a result to pin it to **Your GIFs** at the top, which wraps onto new rows instead of
scrolling sideways. The shelf also accepts image clips: open one
in the preview pane and press the GIF button in its header. **Add GIF file** imports a `.gif`
from disk; the file is validated, copied into the media folder and added to the shelf like a
starred result. The search box filters **Your GIFs** as well as the GIPHY results, and clears with
the **×** beside it. The **Preview size** slider matches
the history grid's card-size control, and the choice is remembered.

The **Loop** toggle above the grid decides whether previews keep animating: on, every GIF loops
continuously; off, each one holds a still frame and plays only while the pointer is over it. Every
preview can pause this way: GIPHY results use the static rendition from the API, clip-backed
favourites use their stored thumbnail, and imported GIF files (plus favourites saved before the
static rendition existed) are drawn to a canvas that holds its current frame until hovered. History
views follow the same hover behaviour, so a screen full of GIFs is not a screen full of motion.
Animated clips also carry a small GIF badge in the top-left corner of their card.

## Adding clips by hand

**Add clip** in the toolbar opens a small **New clip** window: type or paste the text, optionally
with the Markdown buttons (bold, italic, inline code, heading, bullet list, link), then **Save clip**
or `Ctrl+Enter`. The text is stored as a clip without touching the clipboard, so the capture thread
never sees it and no second row appears. The window keeps its draft when it loses focus, and closing
it discards the draft. Whether the text is Markdown is decided when it is stored: a heading, fence,
quote, list, table, link or paired emphasis marks the clip as Markdown, and it gets a **Markdown**
badge plus a *Text → Markdown* sub-view and a `type:markdown` (or `type:md`) filter. Everything else
stays plain text.

## Notes and editing

Every clip has a **Note** field in the preview pane. The note saves when the field loses focus (or
with `Ctrl+Enter`), is stored with the clip, and is included in search results; a note icon marks
the clip in list and card views.

For text clips, double-click the preview to edit the body in place. `Ctrl+Enter` saves and `Esc`
cancels. Editing drops any captured HTML/RTF so stale formatting cannot be pasted over the new
text, and refuses to turn one clip into an exact duplicate of another.

## Color picker

The eyedropper next to the settings button hides CopyPaste and samples the next spot you click on
screen. A small topmost magnifier follows the crosshair, zooming the pixels beneath it and showing
the exact pixel and its hex/RGB values; it flips at monitor edges and never takes focus. While a
pick is pending the click is swallowed by a low-level mouse hook, so sampling a colour cannot also
press a button, open a dropdown or pause a video underneath. Esc cancels, and cancelling (or
letting the pick time out) closes the picker without an error message. The sampled `#rrggbb`
value goes on the clipboard, is added to history, and the window returns focused on the **Colors**
view with the new swatch selected.

## Where things live

```
src/                        React frontend
  lib/ipc.ts                the only place `invoke` is called
  lib/types.ts              TypeScript mirrors of the Rust DTOs
  lib/theme.ts              palettes and the CSS custom-property bridge
  hooks/useHistory.ts       list + search state, debounced, subscription to backend events
  hooks/useUpdater.ts       daily update check, download progress and relaunch
  components/               chrome, list, detail pane, settings
src-tauri/src/
  lib.rs                    app wiring: plugins, tray, hotkey, media URI scheme
  clipboard.rs              capture loop and all Win32 clipboard access
  store.rs                  SQLite storage, plain-text payloads, pruning, in-memory index
  storage.rs                data-folder pointer, relocation and cleanup
  ocr.rs                    text recognition for screenshots (Windows.Media.Ocr)
  palette.rs                dominant-colour extraction for screenshot schemes
  giphy.rs                  GIPHY search, URL allow-listing and GIF downloads
  animated.rs               recovers the source animation when a browser copies a GIF
  picker.rs                 screen colour sampling and the eyedropper magnifier
  crypto.rs                 one-time legacy decryption for histories from older builds
  search.rs                 relevance scoring
  media.rs                  image/thumbnail persistence
  paste.rs                  foreground window tracking + synthetic Ctrl+V
```

## Security model

Clips are stored locally in SQLite as plain text. CopyPaste does not encrypt its database, so
anyone who can read the data folder can read the clips in it. Nothing is ever sent anywhere.

Older builds encrypted the payload columns with AES-256-GCM under a DPAPI-wrapped key. On the first
start after upgrading, those rows are decrypted, rewritten as plain text and the `key.bin` file is
deleted. If a legacy row cannot be decrypted it is left alone and reads as empty rather than as
garbage; `key.bin` is kept in that case so a later attempt can retry.

**Deliberate trade-offs, stated plainly:**

- **The database is not protected at rest.** Rely on OS file permissions, an encrypted system
  drive, or a separate Windows account if the machine is shared.
- **Content hashes are stored in the clear.** SHA-256 digests serve as the de-duplication key.
  They are one-way, but equal payloads remain *detectable* to someone reading the database.
  De-duplication is impossible otherwise.
- **Search runs over an in-memory index of clip text.** Search mirrors the first 16 KB of each
  clip's plaintext in memory; for text clips larger than 256 KB only the preview is indexed, so a
  term deep inside a very large paste will not match.
- **The webview serves images through a custom `media` URI scheme**, with a filename allowlist
  rather than a filesystem scope, so the frontend is never granted directory access.

Respecting opt-outs: copies flagged with `ExcludeClipboardContentFromMonitorProcessing`, or whose
`CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` is zero, are ignored. This is how
password managers mark secret material, and the setting is on by default.

## Notable design decisions

- **Polling the clipboard sequence number** (`GetClipboardSequenceNumber`) instead of listening for
  `WM_CLIPBOARDUPDATE`. One cheap syscall per tick costs nothing, and it avoids a second
  message-only window competing with Tauri's event loop.
- **Timestamps are monotonic, not wall-clock.** Every write is stamped strictly greater than the
  current maximum, so "the clip you picked moves to the top" holds even when two writes land in
  the same millisecond.
- **A suppression latch, not an app-name check, prevents feedback loops.** When the app writes to
  the clipboard it records the content hash and drops the next matching capture. Skipping capture
  whenever the window had focus would silently lose anything copied out of the preview pane.
- **Auto-paste is opt-in.** Injecting keystrokes into other applications is behaviour a user should
  choose. Scan codes accompany virtual keys so the synthetic input also works in applications that
  read raw hardware input.

## Feature tracking

`docs/features.json` is the source of truth for which features are in the backlog, planned, in
progress, blocked or shipped. Each entry has an id, a title, an area, a status and optional notes and references to
the files that implement it.

The **CopyPaste Feature Tracker** canvas (`.github/extensions/copypaste-feature-tracker/`) edits
that file from a board view, and its `list_features`, `add_feature`, `update_feature`,
`remove_feature` and `summarize_features` actions let Copilot keep the board current. The canvas
validates the format marker and every record before writing; a file it cannot parse is reported
instead of being overwritten.

## Releases and automatic updates

Pushing a `v*` tag runs [`.github/workflows/release.yml`](./.github/workflows/release.yml), which
runs the backend tests, builds the NSIS installer, signs it, attaches a stable
`CopyPaste_x64-setup.exe` alias and publishes the release with `latest.json`. The app polls
`https://github.com/jamesbuckley-source/CopyPaste/releases/latest/download/latest.json`, verifies
the signature against the public key in `tauri.conf.json`, and installs in place.

Two repository secrets drive the signing, and losing either breaks future updates:

| Secret | Purpose |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | minisign private key used to sign update artefacts |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | password protecting that key |

Keep a backup of the private key: a release signed with a different key is rejected by installed
copies. The update check is on by default; *Settings → Updates* turns it off and offers a manual
check. When a release is waiting, a *What's new* link sits next to *Check now*, and the first launch
after an update offers the same link to the release notes.

## Tests

```powershell
cd src-tauri
cargo test                 # backend suite

cd ..
npm run build              # type-checks the frontend
```

The suite covers legacy encrypted-history migration, plain-text round trips, search ranking
(content and tags), de-duplication, ordering by capture date or last use, retention pruning in days
and hours, archive protection from pruning and clear-history, color-code recognition, Markdown
detection (including the near-misses it must reject), Markdown note rendering with YAML escaping,
CRLF normalisation and filename collisions, clips typed by hand, collection/tag behaviour,
data-folder relocation and cleanup, theme sanitising, text recognition from a rendered
image fixture, media path-traversal rejection, and the `DROPFILES` layout used for file lists.

Two tests are `#[ignore]`d because they touch the live system clipboard:

```powershell
cargo test -- --ignored
```

For a manual pass over capture and classification, open [docs/demo.html](docs/demo.html) in a
browser. It has copy-ready samples for every recognised payload shape — colors in all supported
forms, links, emails, Windows paths, JSON, synthetic secrets, rich text, an image/OCR pair and a
script that creates a demo file list.

## Known limitations

- Windows-only. The key-wrapping layer deliberately fails loudly elsewhere rather than silently
  writing an unprotected key.
- Deleted clips are unlinked, but storage is not securely erased; SSD wear-levelling makes
  guarantees there impossible.
- Search covers the first 16 KB of a clip (see above).
- File clips record paths; if the files move afterwards the entry is stale.

## License

CopyPaste is free software: you can redistribute it and/or modify it under the terms of the
GNU General Public License as published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

CopyPaste is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without
even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
[GNU General Public License](./LICENSE) for more details.

Third-party components keep their own licenses (React, Vite, Tailwind CSS and Tauri are MIT or
Apache-2.0); see `package-lock.json` and `src-tauri/Cargo.lock` for the full set.

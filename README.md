# CopyPaste

A clipboard history manager for Windows: Tauri 2 + React 19 + TypeScript + Tailwind CSS 4.

Everything stays on your machine, and the parts that matter are encrypted at rest.

## What it does

- **Captures** text, images and copied file lists as you copy them.
- **Summons** with a global hotkey (`Alt+V` by default) as a frameless overlay, or from the tray.
- **Searches** as you type, ranked by exact match, word boundaries, position and recency.
- **De-duplicates**: copying the same thing twice promotes the existing entry instead of
  stacking duplicates, and the clip you reuse jumps back to the top.
- **Organises** with pinning, collections and tags.
- **Previews** the full text, the image, or each file path before you commit to using it.
- **Two layouts** — rows for reading text, cards for browsing images — with an adjustable card
  size and a separate interface scale.
- **Pastes back** on demand into whatever you were working in (opt-in; off by default).

## Requirements

| | |
|---|---|
| OS | Windows 10/11 |
| Node | 20+ |
| Rust | 1.77+ (`rustup`) |
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

## Installing

`npm run tauri build` produces a per-user NSIS installer:

```
src-tauri/target/release/bundle/nsis/CopyPaste_0.1.0_x64-setup.exe
```

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
- **One instance at a time.** Launching again from the Start Menu raises the running instance
  rather than starting a second copy, which would otherwise add a duplicate tray icon and fail
  to claim the global hotkey a second time.
- **Launch at login** is a setting; when enabled the app registers itself with the Windows
  startup list and returns after a reboot, still hidden.

## Keyboard

| Key | Action |
|---|---|
| `Alt+V` | Show / hide the overlay (configurable) |
| `↑` `↓` | Move the selection |
| `PgUp` `PgDn` `Home` `End` | Jump through the list |
| `Enter` | Copy the selected clip, and paste it if auto-paste is on |
| `Ctrl+P` | Pin / unpin |
| `Del` | Delete the selected clip (only while the search box is empty, so it can still edit text) |
| `Esc` | Clear the search, leave settings, or hide the window |

## Appearance

*Settings → Appearance* controls:

| Setting | Default | Notes |
|---|---|---|
| Show in the taskbar | **on** | Off makes the app tray-only |
| Layout | List | Rows, or cards for a thumbnail grid |
| Card size | 100% | Also adjustable with − / + in the toolbar |
| Text scale | 100% | Scales the whole interface, 70–200% |
| Preview pane | shown | Collapse it with the chevron at the pane's top-left |

Interface scaling uses CSS `zoom` on `#root`, which re-lays-out instead of resampling, so text
stays crisp. The layout fills the window at every scale — zoomed elements resolve their
percentages against an already-divided containing block, so nothing else may compensate.

In card view the grid takes the detail pane's width, so more columns fit; switch back to List to
edit tags and collections.

## Where things live

```
src/                        React frontend
  lib/ipc.ts                the only place `invoke` is called
  lib/types.ts              TypeScript mirrors of the Rust DTOs
  hooks/useHistory.ts       list + search state, debounced, subscription to backend events
  components/               chrome, list, detail pane, settings
src-tauri/src/
  lib.rs                    app wiring: plugins, tray, hotkey, media URI scheme
  clipboard.rs              capture loop and all Win32 clipboard access
  store.rs                  SQLite storage, encryption, pruning, in-memory index
  crypto.rs                 DPAPI key wrapping + AES-256-GCM sealing
  search.rs                 relevance scoring
  media.rs                  image/thumbnail persistence
  paste.rs                  foreground window tracking + synthetic Ctrl+V
```

## Security model

Two independent layers:

1. **Key wrapping.** A random 256-bit data key is generated once and sealed with Windows DPAPI
   (`CryptProtectData`), which derives its protection from your logged-on credentials. Only the
   sealed blob is written to disk. Copying `key.bin` to another machine or user account yields
   nothing, which is why there is no passphrase prompt.
2. **Record encryption.** `preview`, `body` and `files` columns are each sealed with
   AES-256-GCM under a fresh random 96-bit nonce. GCM is authenticated, so a tampered ciphertext
   is rejected on read rather than decrypted into garbage.

Non-sensitive bookkeeping (kind, sizes, timestamps, pinned flag) stays plaintext so it can be
sorted and counted without decrypting anything.

**Deliberate trade-offs, stated plainly:**

- **Content hashes are stored in the clear.** SHA-256 digests serve as the de-duplication key.
  They are one-way, but equal payloads remain *detectable* to someone reading the database.
  De-duplication is impossible otherwise.
- **Search runs over an in-memory index of decrypted text.** SQLite cannot index ciphertext, and
  the alternative (SQLCipher) would pull a native OpenSSL build into the toolchain. Search mirrors
  the first 16 KB of each clip's plaintext in memory; for text clips larger than 256 KB only the
  preview is indexed, so a term deep inside a very large paste will not match.
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

## Tests

```powershell
cd src-tauri
cargo test                 # 73 backend tests

cd ..
npm run build              # type-checks the frontend
```

The suite covers the encryption round trip (including tamper and wrong-key rejection), DPAPI key
persistence, search ranking, de-duplication, retention pruning, collection/tag behaviour, media
path-traversal rejection, and the `DROPFILES` layout used for file lists.

Two tests are `#[ignore]`d because they touch the live system clipboard:

```powershell
cargo test -- --ignored
```

## Known limitations

- Windows-only. The key-wrapping layer deliberately fails loudly elsewhere rather than silently
  writing an unprotected key.
- Deleted clips are unlinked, but storage is not securely erased; SSD wear-levelling makes
  guarantees there impossible.
- Search covers the first 16 KB of a clip (see above).
- File clips record paths; if the files move afterwards the entry is stale.

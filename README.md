# Windman

A macOS launcher for applications and shell commands, with built-in terminal tabs.

```sh
cargo run
# or build a .app bundle
./scripts/bundle.sh
open dist/Windman.app
```

- One movable, resizable palette-style window (minimum 420×320) with Dark Aqua appearance and a translucent blurred backdrop.
- Runs as a `UIElement` accessory app: no Dock icon and no Cmd+Tab entry.
- Remembers its last screen position and size.
- Escape closes the launcher; **Cmd+Space** toggles it: opens/focuses when inactive, closes when active. The app stays running in the background.
- Cmd+Enter reveals the selected app in Finder instead of launching it.
- Launching an app closes the launcher.
- Type to search `.app` bundles in `/Applications`, `/System/Applications`, and `~/Applications`, including subfolders such as Utilities; matched characters are bold in result titles.
- App icons come from the macOS workspace icon API (falling back to bundle icon metadata), cached as PNGs.
- Enter opens the selected app; no app match becomes `Run “…”` using `$SHELL -l -i -c`, so login and interactive startup files (aliases, colors, PATH) apply. When the query matches a recent command, the typed command and command history rank above app matches.
- `>` forces command mode.
- Command tabs are directly under the input: running commands plus at most one finished command. Commands that exit without visible output close their tabs automatically; whitespace and terminal control sequences do not keep them open. Output in scrollback is preserved. Reopening the window or clearing the input restores the last selected command tab, falling back to the latest remaining tab if it was closed. Typing or arrow navigation shows the result list.
- Running a command selects its tab, enters its terminal, and shows the live PTY screen. When the program exits, focus returns to the search input. Running tabs show a dark-red dot; finished tabs have no dot.
- If the selected command exits without visible output while terminal input is selected and search is empty, the launcher closes automatically, including when the command activates another app.
- Terminals run on `portable-pty` with Ghostty's emulator core (`ghostty-vt` / libghostty-vt): ANSI colors, bold/italic/underline, inverse, alternate screen, cursor, and truecolor.
- A bare **Cmd** tap toggles focus between the search input and the selected terminal; Cmd chords (Cmd+C, Cmd+Space, …) do not toggle.
- Focus is always in the search input or a running terminal. Clicking finished output, selecting a tab, or closing a tab focuses the search input.
- **Tab** / **Shift+Tab** in the search input cycle command tabs forward / backward while keeping focus in the search input.
- The tab strip scrolls horizontally with the trackpad or mouse wheel when it overflows. Selecting or restoring a tab scrolls it into view.
- Drag a command tab onto another tab to reorder it. The selected command and tab order are retained when the window reopens.
- In terminal focus, typed text, Enter, Tab, Backspace, Delete, Home/End, Page Up/Down, F1–F12, arrows (with Shift/Alt/Ctrl), Ctrl combos and Cmd+V paste are encoded into the PTY. Escape goes to the terminal and does not close the window.
- Mouse press/release/move and wheel are forwarded to programs that enable mouse tracking; otherwise the wheel scrolls terminal scrollback.
- OSC 52 clipboard writes from terminal programs go to the system clipboard.
- Arithmetic like `2+2*3` shows a calculator result; Enter copies it and closes the launcher.
- Currency queries like `10 usd in eur` or `10$ in rub` show a converted amount; Enter copies it. Rates come from `open.er-api.com`, cached for 24 hours in `rates.json`.
- Recent apps and commands are persisted and ranked first while searching.
- Cursor blinks white; selection/search/active accents are white/gray.
- Editing supports Cmd/Alt+Left/Right, Shift variants, Home/End, Cmd/Alt+Backspace/Delete, and Shift+Home/End.
- Long input scrolls horizontally to keep the caret visible while typing, selecting, or navigating.
- Tab `×` kills a command. Cmd+Q is disabled so it cannot quit the app or interrupt running commands.

Prototype limits: no text selection. Terminal columns/rows follow the window size and font.

## Settings

`~/Library/Application Support/windman/config.json` is created on first run:

```json
{
  "terminal_font_family": "Menlo",
  "terminal_font_size": 11
}
```

- Applies to terminal text; use a monospace family.
- Size is clamped to 8–24; columns/rows and the PTY size adapt to it.
- Type `config`, `settings`, or `preferences` and press Enter, or press **Cmd+,**, to open the file.
- Restart Windman after editing.

## Distribution

### App icon

Generate the app's magnifying-glass icon without a source image:

```sh
./scripts/icon.sh
./scripts/bundle.sh
```

The script draws the same magnifying glass as the search field on a dark rounded tile,
rendering each standard and Retina size directly into `assets/Windman.icns`.
Bundle and release builds regenerate it automatically and register it in `Info.plist` before signing.
An optional argument selects a different output path; set `APP_ICON` when building to use a custom `.icns` file.

### Builds

`./scripts/bundle.sh` builds a debug, ad-hoc-signed app for local testing.

`./scripts/release.sh` builds a signed release app:

```sh
./scripts/release.sh                          # arm64 release, signed
./scripts/release.sh --universal              # arm64 + x86_64 via lipo
./scripts/release.sh --universal --dmg        # + dist/Windman-<version>.dmg
NOTARY_PROFILE=windman-notary ./scripts/release.sh --universal --dmg --notarize
```

Signing identity is taken from `SIGN_IDENTITY`, otherwise the first Developer ID Application identity, then Apple Development, then ad-hoc. Only a Developer ID Application identity can notarize.

One-time notarization setup:

```sh
xcrun notarytool store-credentials windman-notary \
    --apple-id you@example.com --team-id TEAMID --password APP_PASSWORD
```

Without notarization, Gatekeeper blocks downloaded copies; users must right-click → Open or remove the quarantine attribute.

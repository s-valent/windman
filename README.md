# Windman

A native macOS menu-bar prototype for focus follows mouse with protected raising.
Requires macOS 14 or newer and Swift 6 or newer to build.

## Run

```sh
bash scripts/build-app.sh
open build/Windman.app
```

Use **Grant Accessibility Access…** in the cursor-shaped menu-bar item, then enable
Windman under System Settings → Privacy & Security → Accessibility. Permission is
rechecked on application activation and when opening Windman's menu.

The menu provides a pause toggle. Slow entry into another window focuses it
immediately. Fast crossings leave focus and stacking unchanged until the pointer
slows down or stops for 70 ms. There is no configurable delay. The enabled state persists
between launches. Dragging and using Windman's menu suspend focus
changes. The app runs without a Dock icon.

Entering an exposed desktop area focuses Finder's desktop without raising its
folder windows. The menu bar, Dock, and pop-ups remain separate from the desktop.

Clicks take precedence over automatic focus. Button releases, completed drags,
menu dismissal, and starting or resuming Windman do not count as entering a window.
Mouse button events also cancel pending focus changes, so opening a pop-up does
not refocus its parent window.

## Window rule

For a hovered window A, examine the normal-layer windows above it. Any window
whose entire rectangular bounds are inside A is carried forward, staying above A.
All carried windows retain their relative order. A receives keyboard focus.

For example, with front-to-back order `B → C → A → D`, if A contains B but only
partly overlaps C, the result is `B → A → C → D`.

Window positions and sizes are not changed. Containment uses full window bounds,
not shadows, transparency, or the portion clipped by display edges. Only on-screen
windows belonging to currently active Spaces participate. Dialogs, including
Accessibility permission dialogs, are protected by the same containment rule.
Menus, desktop elements, and elevated floating windows are
not normal-layer targets. Unsupported Accessibility windows are reported in the
menu; the group is resolved before raising begins.

## Implementation

- Swift/AppKit menu-bar app driven by local/global mouse event monitors, with no
  recurring polling timer. Movement bursts are coalesced within a main-queue turn.
  Speed is measured from event positions and timestamps in display points.
- Motion at 700 points/second or faster defers a new hover. Slowing below 300
  points/second for 20 ms of movement permits focus immediately; an abrupt stop
  uses a reschedulable, one-shot 70 ms check. Clicks, suspension, desktop changes,
  and returning to the already-handled window cancel that check. No timer remains
  active after the pointer settles.
- Workspace notifications reset tracking on Space, display, and session changes.
  Desktop and window snapshots are queried when entering a different window, rather
  than periodically while idle.
- A pure hover tracker owns entry and click state, returning an explicit decision
  to focus, wait for a stop, wait for restoration, or interrupt. The controller
  handles event delivery, the one-shot stop check, and serialized execution.
- Desktop and window targets share one execution path. Target planning reuses one
  decoded window list; transition verification always reads current state.
- Pointer movement during a transition is coalesced to the latest position. The
  current transition finishes restoring protected windows before the next hover
  is processed, so fast movement cannot abandon a partially reordered stack.
- Core Graphics supplies the front-to-back window list and geometry.
- Private SkyLight hit testing identifies the actual window under the pointer.
- Space membership is checked explicitly. Focus changes pause during desktop
  animations, and hover tracking resets when the active Space changes.
- The desired stack is compared with the current order before any changes. If it
  already matches, only keyboard focus changes; protected windows are not activated
  or checked for raise support.
- When ordering must change, the plan selects the smallest frontmost portion of
  the desired group that needs raising and raises it before requesting keyboard
  focus. If raising already supplied focus, no separate focus action is sent.
  It checks again after focusing, since some applications change their own stacking
  order during activation.
- When an app advertises `AXRaise` but rejects it (as System Settings does), Windman
  selects its main window and activates the app instead.
- If a successful raise call has no effect, Windman focuses that window and retries
  the raise before restoring focus to the hovered target.
- For the hovered target, raise and any necessary focus/retry requests are sent
  without an intervening completion wait. Focus is verified after ordering, rather
  than holding up the raise while waiting for activation to finish.
- Operation-scoped Accessibility observers and workspace activation notifications
  drive focus/order verification. A one-shot deadline performs a final check for
  apps that omit notifications; there are no repeated verification sleeps.
- Raising waits for the actual window order to change rather than assuming an
  Accessibility success return means the animation and activation have completed.
  Already-front windows and already-focused targets are skipped. Once a hover is
  accepted, there are no fixed focus delays beyond the same-app private event workaround.
- Private SkyLight process/key-window events restore the target's keyboard focus.
  The final stacking order and focused window are checked before reporting success.
- Private symbols are resolved at runtime; missing APIs disable processing with a
  status message rather than a launch-time linker failure.

Direct private ordering of arbitrary foreign windows is privilege-dependent. This
prototype uses Accessibility for ordering and does not require a Dock helper.
Actual reordering through the Accessibility backend remains sequential and can
visibly cover protected windows before restoring them. Focus-only transitions do
not rebuild an already-correct stack. Fully flicker-free reordering remains
unresolved. Suspending updates on Windman's own WindowServer
connection does not make changes to other applications atomic.

The required backend must insert a window below a protected window or lower
intervening windows, preserving the protected order throughout the transition.
Direct relative ordering and the tested private bridge did not reorder foreign
windows on this SIP-enabled system. SIP remains enabled; no Dock helper is installed.
Apps may also react to activation by changing their own window order.
Full-screen transitions, Mission Control, Stage Manager, and app-specific focus
behavior need interactive compatibility testing.

## Development

```sh
just fix
just test
just check
just build
build/Windman.app/Contents/MacOS/Windman --diagnose
```

Without `just`, use `xcrun swift-format`, `bash scripts/test.sh`, and the build script
directly. The test script explicitly loads the Swift Testing macro plugin when it
is installed in the Swift 6.4 command-line toolchain's nested plugin directory.
Diagnostics report private-symbol availability, the diagnostic process's Accessibility permission, and the
number of visible normal-layer windows without changing focus. Runtime errors also
appear in Console under subsystem `local.windman`.

A diagnostic launched from a trusted terminal can inherit that terminal's access
even when the menu-bar app lacks permission. Check the menu-bar status for the
running app's grant, especially after rebuilding an ad-hoc-signed binary.

Diagnostics also list active Space IDs, animation state, window IDs, application
names, and bounds. To reproduce one
focus transition without running the mouse tracker, use a listed window ID:

```sh
build/Windman.app/Contents/MacOS/Windman --focus-window 123 --trace
```

This command changes focus and stacking, prints the protected windows and each
transition, verifies the result, and exits with a nonzero status if it fails.

The build script uses the local Apple Development certificate when exactly one is
available, preserving the signing identity across rebuilds. Set
`WINDMAN_SIGNING_IDENTITY` to select an identity explicitly. Otherwise it falls back
to ad-hoc signing, which can require granting Accessibility access again after a
rebuild. Switching from ad-hoc signing to a certificate can also require a new grant.

### Interactive check

Place a small window fully inside a larger window, and another window partly
outside it. Hover over an exposed part of the large window: the small window
should remain above it, the partial overlap should go behind it, and typing should
reach the large window. Also check windows from the same app, another display,
dragging, and immediate switching when entering another window.

See `THIRD_PARTY_NOTICES.md` for the private focus protocol's attribution.

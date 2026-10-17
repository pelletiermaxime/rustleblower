# Connection and application lifecycle

Rustleblower listens for local daemon sessions, shows its heartbeat/version/rule count,
and keeps serving permissions when its main window is hidden in the system tray.

## Sub-features

- `connection-subscribe`: daemon configuration is echoed unchanged.
- `connection-heartbeat`: Ping is acknowledged quickly and refreshes status.
- `connection-alert`: the latest daemon alert appears in the dashboard.
- `connection-reconnect`: same-name replacement invalidates only that node's old requests/commands; distinct names retain independent sessions.
- `connection-owned-socket`: existing endpoints are never overwritten.
- `lifecycle-tray-quit`: closing hides the dashboard to a registered system tray; activation/menu Open restores it, and Quit stops the server. Missing tray support falls back to minimize; lost support restores a hidden dashboard.

## How to get to it (user POV)

- Launch Rustleblower from the desktop or terminal.
- The daemon connects to the configured Unix socket automatically.
- Read the dashboard status; close it to the tray beside the clock. Click the
  icon or choose Open Rustleblower to return. Use Quit Rustleblower in the
  dashboard or tray menu to exit. Permission windows continue independently.

## Driving it with native controls and protocol tests

Preconditions: use the helpers' isolated socket and database; leave system
opensnitch-ui and opensnitchd alone.

- **Connection display.** Run `python scripts/verify-native.py run --feature history`.
  Inspect the populated dashboard's synthetic daemon version/name and require
  successful Ping RPCs in `fixture-report.json`.
- **Protocol lifecycle.** Run `python scripts/cargo.py test --no-default-features --locked`.
  Require subscription/ping/alert, reconnect, stream-close, shutdown, and existing
  endpoint protection tests to pass. `tests/uds_authority.rs` also verifies the
  daemon's raw socket-path authority and C-core's encoded form through actual
  HTTP/2 frames. Socket cleanup is asserted after joining.
- **Background lifecycle.** Run `python scripts/verify-headless.py --feature tray`.
  Require four Kit JSON checkpoints, a destroyed dashboard, an input decision while
  hidden, retained Nodes page on reopen, no duplicate on repeated activation,
  automatic restoration after host loss and dashboard Quit.
  The close-handler boundary and tray channel are invoked directly; this does
  not prove compositor close input, the test platform does not implement
  minimize, and D-Bus registration requires native proof.
- **Compositor/tray lifecycle.** Run `python scripts/verify-native.py run --feature tray`.
  Require owned Alt+F4 to destroy the dashboard, actual SNI Activate and dbusmenu
  Open to restore it, keyboard permission decision while hidden, persisted
  decision and tray Quit's clean exit before cleanup. The actual tray service
  must belong to the owned PID. This proves protocol callbacks, not a pointer
  click on the desktop panel icon. Never restart the shared tray host for a test.
  `tray-icons.json` must contain 16/22/24 px tiny colour artwork and 32 px compact
  colour artwork. Every actual ARGB payload must match the checked-in asset
  manifest, and IconName must be empty to avoid overriding the size-specific
  drawings. Full mascot launcher assets are checked separately by package
  verification; this Wayland flow does not prove the X11 window-icon property.

## Gotchas

- Protocol regressions send Ping before Subscribe, verify that early events are
  persisted under the subscribed name without a loss warning, and replay them
  to check deduplication. They also check the 32-batch startup bound, disconnect
  cleanup, and rejection of reports from a superseded session.

- The dashboard entity and permission loop outlive its native window.
- The desktop must support StatusNotifierItem to show the tray icon. Autostart is opt-in through Startup preferences below. A crash can leave a stale socket;
  establish ownership before removing any endpoint manually.
- Local Unix sockets only. Up to 32 distinct named sessions are supported;
  TCP/TLS and remote peer identity remain outside this slice. See [Nodes](nodes.md).
- A live server with stale daemon heartbeat is different from an active daemon.

## Startup preferences

Pinned source: utils/xdg.py::Autostart, preferences/settings.py and service.py
tray autostart action. The native equivalent is **Startup…** in the dashboard
or **Startup preferences…** in its tray menu. Checkbox changes are staged;
Save applies, Close preserves the old state and Reload discards the draft.
The same owned window is reused on repeated opens. File discovery, reads and
atomic writes run on GPUI's background executor, independently of AskRule.

The effective entry follows XDG_CONFIG_HOME then ordered XDG_CONFIG_DIRS
(default ~/.config and /etc/xdg). Relative XDG roots are ignored. The user
filename is dev.rustleblower.Rustleblower.desktop. A disabled managed override
uses Hidden=true, including when a system entry exists. No file is changed merely
by launching Rustleblower. Existing OpenSnitch entries remain untouched. Only
managed user files can be replaced; foreign/symlink/invalid/oversized user files
report an error. Fingerprint checks reject stale saves; they are not a generic
filesystem compare-and-swap against concurrent external writers.

The launch command preserves configured socket, database, prompt timeout and
fallback; relative endpoint paths become absolute at configuration time. Saved
retention remains authoritative. The original AppImage path is used only when
the current executable belongs to APPDIR; unrelated inherited APPIMAGE values
are ignored. Moving the chosen binary/AppImage requires saving again. Desktop
Exec quoting retains literal arguments, without a shell. Percent-bearing binary
paths use /usr/bin/env -- because actual GIO preflight rejects the ordinary
escaped first token; argument decoding is independently exercised.

Bounds: 16 system roots, 4096-byte paths/arguments, 32-KiB command, 64-KiB desktop
entry, one I/O task per window and no per-event settings reads. Existing entry
constraints such as OnlyShowIn/TryExec are not interpreted as observed login
scheduler behavior; UI status describes the enabled entry. Generated entries
do not include those restrictions. Missing/moved executable, '=' in executable
name, non-UTF-8/control-character paths and malformed state produce feedback.

Run `python scripts/verify-headless.py --feature startup` for six frames using
actual checkbox/save/close/reload input, durable enabled/disabled state across
reopen, no file after staged Close, synthetic tray Startup action, real AskRule
while saving, external stale-entry refusal with retained selection and Reload.
Its wrapper supplies owned XDG_CONFIG_HOME and XDG_CONFIG_DIRS beneath evidence;
the journey refuses to run against any other paths. It changes no real startup
entry and never executes its generated Rustleblower command.

`tests/startup.rs` has three precedence/preservation/path cases. For independent
decoding proof, set RUSTLEBLOWER_STARTUP_EVIDENCE to a new absolute fixture path
while running that test, then run:

```sh
python scripts/verify-startup-entry.py --fixtures /absolute/owned-fixture --evidence /absolute/new-proof
```

Require desktop-file-validate and gio, passed result.json with exact arguments,
and the fixed recorder source checked by the helper. It clears display/session
bus addresses and runs no daemon or real app. It tests spaces, quote, dollar,
backtick, percent, backslash and Unicode in actual argv. Retain the first failed
ordinary percent-path attempt and its successful env-based replacement.

Sources: [XDG Autostart](https://specifications.freedesktop.org/autostart/latest/)
and [Desktop Exec](https://specifications.freedesktop.org/desktop-entry/latest/exec-variables.html).
Real login
scheduling, this tray entry's actual D-Bus callback, native compositor/keyboard
behavior and packaged AppImage startup remain separate proof boundaries.

## Remembered main window

Pinned events/dialog.py restores/saves geometry, maximized state and last tab
on close/hide. Rustleblower automatically remembers its restore rectangle,
windowed/maximized/fullscreen mode and one of seven pages. Warm tray reopening
uses the retained state; a new process reads `<database>.window.json`. Current
monitor bounds fit the saved rectangle, including negative-origin monitors and
disconnected/offscreen cases. Wayland decides actual window origin.

The document and reads are capped at 4 KiB. No other feature settings or daemon
policy are included. Corrupt/future/oversized/symlink files are preserved with
feedback and defaults; a stale-write error stops further automatic writes.
Restore can be retried by restarting after the owned layout file is repaired or
removed. Existing unrelated files must not be removed. History export reserves
this filename even with replacement enabled.

A hidden bootstrap creates the dashboard/permission owner before loading. Its
separate title prevents native verifiers adopting an unmapped bootstrap as the
visible dashboard. The loader runs off the UI thread; after 250 ms default
geometry appears, with late results ignored. A pending prompt prevents initial
main-window focus request. One background task, one pending watch snapshot and
one latest status retain bounded state. Samples compare every 50 ms; disk writes
wait for 500 ms stability. Close/Quit flush the latest sample. Quit shuts down
permissions/backend immediately, then waits at most 500 ms for layout save.
A process kill/power loss before the latest write is not covered.

Run `python scripts/verify-headless.py --feature window-state` for thirteen frames.
Require current-display fitting and saved Nodes page; actual Rules navigation;
25 GPUI geometry requests with callback shim yielding one write; close/Open and
permission while hidden; a permission before held load-result delivery, default
startup timeout and refusal of late data; cold restore; stopped stale writes;
restart after explicit fixture repair; actual Quit while a request is pending,
its once-only fallback and final page flush. The load delivery gate is an owned
fixture boundary, not a deliberately stalled filesystem. Geometry requests and
close/Open are GPUI/synthetic boundaries, not native compositor gestures.

Three `tests/window_state.rs` cases cover 21 page/mode roundtrips, unchanged
siblings, finite/invalid bounds, negative/disconnected/small monitor fitting,
stale/corrupt/future/oversized/symlink files.
Rebuild desktop after core tests, then for actual KDE compositor proof run:

```sh
python scripts/verify-native.py doctor
python scripts/verify-native.py run --feature window-state
python scripts/verify-native.py run --feature window-restore --restore-state /absolute/prior-passed-evidence/history.sqlite3.window.json
```

The second run validates the prior passed/cleaned report and exact saved-file
SHA before copying only that state into its new owned database directory.
Require real native resize/maximize and expanded geometry on the first PID; a
fresh PID must restore maximized geometry and visible Nodes-page text, then
native unmaximize must recover the prior normal size within three logical pixels.
Fractional KDE geometry is parsed by labelled fields, excluding the window UUID.
Both reports must have 430 history rows, zero policy commands and all cleanup
flags. Four PNGs plus window-layout.json and the persisted state show the result.
The adapter uses FerryDock's shared input lock and exact owned PID/executable.
It never changes real startup/theme files, daemon/socket/firewall or other windows.

The final native `--feature tray` regression uses the same desktop binary and
proves actual close, SNI Activate and dbusmenu Open/Quit callbacks, plus permission
input while the dashboard is absent. Require 431 history rows, one persisted
decision, zero policy commands and complete cleanup. This is callback proof,
not a pointer click on the desktop panel's tray icon.

Native
fullscreen, X11 restoration, physical monitor-disconnect/focus variants, separate
column/splitter/filter state, package/RSS and final whole-app UI/UX remain separate.

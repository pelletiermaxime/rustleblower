---
name: verify-rustleblower
description: Verify Rustleblower's native GPUI permission prompts, history, daemon protocol, and resource bounds. Use real WGPU screenshots and GPUI input in the background, or FerryDock-derived owned Wayland/X11 foreground controls. Preserve evidence and scoped cleanup.
---

# Verify Rustleblower

Work from the repository root. Read [features/README.md](features/README.md) and
the selected feature first. This is a native desktop application; browser tools
cannot drive it. Scripts follow FerryDock's background GPU and foreground native
verification patterns. The native adapter is adapted from FerryDock's
`scripts/verify-log-performance.py`, with its MIT notice under `LICENSES/`.

## Launch

For the normal background development check, which does not take desktop focus:

```sh
python scripts/verify-headless.py
```

Use `python scripts/verify-headless.py --feature popup-preferences` for the
focused Select/Checkbox/save journey. It verifies the actual controls and file
reopen, retains thirteen PNGs and uses the same GPU/ownership/cleanup checks. The
default full flow additionally verifies saved defaults in real gRPC replies and
immediate once-only fallback when popups are disabled.
The focused journey also sets four advanced condition defaults and expanded
options, rejects zero seconds without publishing staged settings, and saves
five seconds plus Reject once. Its assertions independently reopen the file.
It also saves placement, opens production popups in all five positions, checks
their virtual geometry and answers by input, including Tab/Space. A queued
corner-to-center transition must replace the old popup and retain its response.

Use `python scripts/verify-headless.py --feature countdown` for a four-frame
permission-review journey. It opens real AskRule RPCs, pauses the ordinary
two-second timer through pointer input and Tab, waits beyond it, then requires
actual Allow/Deny replies and two persisted decisions. The next request must
start with a fresh countdown. The full flow also checks initially expanded review
and an untouched one-second fallback with advanced display disabled.

Use `python scripts/verify-headless.py --feature checksums` for the nine-frame
[checksum warning/review](features/checksums.md) journey. It verifies MD5/SHA1,
page navigation, confirm/cancel, held ACK without optimistic cache changes,
permission input during that hold, stop, ERROR/uncertainty, explicit fresh
inventory and updates across all remaining pages with preserved metadata.

Use `python scripts/verify-headless.py --feature history-tools` for the ten-frame
[history conditions/table/export](features/history.md) journey. It combines UID
and command conditions, rejects an invalid port, saves/reopens selected columns,
sorts through the actual Kit header, exports all matches to CSV/JSON Lines,
checks existing-file preservation, copies a matching page and pages sorted results.

Use `python scripts/verify-headless.py --feature statistics` for the ten-frame
[statistics](features/statistics.md) journey: all categories, actual chart and
selectors, frozen paging, CSV/JSON/clipboard, exact history drilldown and confirmed
selected/category resets through the production gRPC counter path.

Use `python scripts/verify-headless.py --feature nodes` for the eleven-frame
[Nodes](features/nodes.md) journey: two live gRPC fixtures, actual node/rule
selection, reviewed interception, held ACK while another node receives a prompt,
configuration JSON review/save, ERROR preservation and search.

Use `python scripts/verify-headless.py --feature daemon-preferences` for the
eleven-frame [daemon preferences](features/daemon-preferences.md) journey: typed
controls in every group, invalid-limit rejection, changed-field review, complete
JSON roundtrip, exact unknown-number preservation, held ACK, independent prompt,
confirmed persistence and rejected-save preservation.

This launches the locked `headless-tests` flow, removes display environment
variables only in its child, and supplies a real WGPU renderer. It creates its
own virtual windows, temporary socket/database, and unique evidence directory.
Missing GPU support fails the test; there is no no-op-renderer fallback.
The initial data enters through production backend APIs. Product actions use
GPUI pointer/keyboard events, never application state setters.

For a foreground desktop check:

```sh
python scripts/cargo.py build --locked
python scripts/verify-native.py doctor
python scripts/verify-native.py run --feature history
python scripts/verify-native.py run --feature prompts
python scripts/verify-native.py run --feature prompts --decision allow
python scripts/verify-native.py run --feature countdown
python scripts/verify-native.py run --feature checksums
python scripts/verify-native.py run --feature history-tools
python scripts/verify-native.py run --feature statistics
python scripts/verify-native.py run --feature nodes
python scripts/verify-native.py run --feature daemon-preferences
```

For explicit X11 placement proof, use
`python scripts/verify-native.py run --feature placement --backend x11 --position bottom-right`
and repeat with center/top-right/top-left/bottom-left. The verifier reads geometry;
it never sets the product window's position. Require `geometry.json` to pass its
strict frame-aware tolerance and require the actual input decision before the
fallback could fire. Native Wayland controls placement; this feature refuses to
claim native Wayland corner proof. Its preference selector is disabled there.
Build desktop after any no-default-features core test, which can replace the
shared top-level binary path.

The default backend is native Wayland when available. The demonstrated Wayland
adapter requires KDE/KWin, `kdotool`, `qdbus6`, Spectacle, Tesseract, `ydotool`,
`ydotoold`, and access to `/dev/uinput`. For an explicitly requested X11 check,
use `--backend x11 --display :0`; this requires xdotool and ImageMagick. Never
silently substitute XWayland for a failed native Wayland check.

Each native run owns its app PID, input daemon, temporary socket, synthetic
daemon client, and database. It never adopts the user's existing Rustleblower or
connects to `/tmp/osui.sock`. The input adapter shares FerryDock's advisory input
lock to prevent simultaneous foreground drivers. Do not manually drive the
desktop during the foreground run. Shutdown/cleanup happens in `finally`.

Native countdown proof uses an isolated five-second launch timeout. After Tab,
it waits another 5.2 seconds, requires OCR of Reviewing/paused on the still-owned
permission window and then Space's actual reply, with latency greater than five
seconds. The daemon fixture retains its normal 120-second RPC limit. That limit
still applies while the ordinary countdown is paused; the core suite separately
verifies expiry before a shorter RPC limit and captured once-only fallbacks.

Native checksum proof opens a separate owned review by Tab/Space, requires OCR
of saved/reported values, then returns to the permission window to answer once.
The report must contain no rule commands. This is window/read/input proof;
native update controls remain separate from GPU/protocol/persistence proof.

Native history-tools proof opens the window from the focused main search, chooses
UID/Equals with Enter on the pinned Select, types 9999 and adds/applies it through
keyboard input. Main-window OCR must show No matching events against fixture UID
1000; require persistence and full cleanup. Enter, not Space, opens this Select.
Keys that change focus/selection need a rendered frame before the next action.
This does not prove native file pickers, table resizing or export buttons.

Native statistics proof uses Ctrl+3, requires painted totals and fixture
applications, then Ctrl+F plus an absent label must produce No matching daemon
counters. Require three persisted application counters summing to 430 and zero
policy commands. This does not prove native reset, chart or export controls.

Native Nodes proof uses Ctrl+4 and Ctrl+F. Require reported name/interception
OCR, empty-search OCR, one persisted metadata row with the expected kernel,
configuration and running state, zero policy commands, 430 events and all cleanup
flags. It does not prove native interception/configuration controls.

Native daemon-preferences proof opens the selected node's editor with keyboard
input, changes Default action through the actual Select, reviews before/after
values and confirms. Require saved-feedback OCR, exactly one successful
CHANGE_CONFIG (protobuf action 6), independently persisted JSON containing only
the changed action, 430 events and complete cleanup. The fixture acknowledges
configuration only under its explicit `--allow-node-config` flag and never writes
system files. This proves the native default-action edit path, not every field,
actual remote TLS transport or enforcement by the user's daemon.

Core transport/storage checks run without the desktop:

```sh
python scripts/cargo.py test --no-default-features --locked
```

These bind disposable Unix sockets and use generated gRPC clients against the
production server. The Python foreground fixture uses installed OpenSnitch
bindings with their natural UDS authority; do not force `localhost`, which masks
OpenSnitch transport compatibility failures. `tests/uds_authority.rs` also sends
raw socket-path and encoded authorities and verifies rejection outside the UI
protocol. The scoped transport patch is documented in `docs/transport-compatibility.md`.
The Cargo wrapper prepares its checksum-verified source under ignored `.deps/`
before dependency resolution; the background verification wrapper uses it too.
The fixture sends its initial traffic report before Subscribe to exercise the
real daemon's possible startup ordering on every run.
No test changes firewall policy.

## Doctor

`python scripts/verify-native.py doctor` is read-only: require `ready: true`, the
expected backend, and the built binary SHA-256. A foreground run then verifies
the exact owned PID/executable snapshot/window title and focus before input or capture.
Focus changes fail the run. Rebuild changed source before native tests.

For a completed background run, read its printed `result.json`: require
`status: passed`, `owned_process_exited: true`, and successful `headless.json`
with `real_gpu: true`, `scoped_socket_removed: true`, and every listed PNG.
Inspect `test.log` on failure. A Cargo success without the completed flow report
is insufficient. Commands may require sandbox access to sockets, GPU devices,
the desktop, or the input device; preserve their ownership scope when escalating.

## Drive

`python scripts/verify-headless.py` drives one full flow: first page → Older →
Latest → Denied → Ctrl+F and typed absent query → matching executable → Allow
once by click → Deny once with Tab/Space. It checks resulting page contents and
the backend oneshot replies. Stable Kit IDs are `older`, `latest`, `all`,
`allow`, `deny`, `allow-once`, and `deny-once`.
The flow also reports an oversized event batch and captures the specific
history warning; normal history rendering has no warning. It then opens
Application rules, pages, changes an action, creates a permanent rule, filters,
tests a rejected change, cancels then confirms deletion, and exercises the full
editor's rename, metadata, regex, duration, JSON and duplicate workflows. It also
exports/imports rules, exercises partial failure/retry and explicit replacement,
preserves selected names across pages, stops/resumes bulk changes and applies
all matching pages. A separate permission popup is answered while a bulk reply
is held. These rule actions
use the production gRPC stream with an isolated daemon acknowledgement fixture.
The flow then selects a history row, inspects/copies its connection metadata and
arguments, opens the current matched rule, saves an application+destination draft,
and rejects existing-name collisions in both creation paths. A bounded argument
snapshot exercises explicit truncation feedback.
It also selects a permanent Reject with command/user/port/checksum conditions,
copies live environment/checksum details, checks awaiting-observation feedback
and disabled edits, reports the daemon's rule to confirm it, verifies defaults
reset on the next request, and allows a five-minute network match.
TestPlatform resizes do not deliver compositor callbacks; the harness calls the
documented `Window::bounds_changed` boundary when bounds and viewport differ.
Controls still receive pointer/keyboard/wheel events; it does not set UI state.
It opens Popup preferences from the dashboard, saves a 12-hour executable
default with priority/no logging, reopens the file and verifies the next RPC
reply. Disabling popups then returns deny/once immediately without a queued
request. Selects use fixed-height parent containers because the pinned Kit
component's outer wrapper fills its parent. The harness waits for popover focus
and deferred selection commits before the next input.

The foreground history flow types through Ctrl+F, captures the real window, and
requires OCR of the empty state. The foreground prompt flow finds the owned
permission window, sends Tab then Space, and requires the synthetic daemon to
receive deny/once (or allow/once with `--decision allow`) before the configured 30-second fallback could fire.

Use `python scripts/fake-daemon.py --socket /absolute/owned/ui.sock` only with an
owned app instance. `--burst-batches N --batch-size N` adds synthetic load;
`--prompts N` controls requests and `--report PATH` records replies/latency. The
displayed documentation addresses are never contacted. This fixture creates
history rows and may trigger prompts; use an isolated database.

## Evidence

Background runs retain `result.json`, `headless.json`, `actions.json`, `test.log`,
and forty-eight numbered PNGs under `.verification/headless/<unique-run>/`. Inspect the
history, filtered/empty states, pending prompt, completed prompt, and history
warning frames, application rule browse/change/create/reject/delete frames, and
full rule editor, duplicate, match JSON, invalid-duration, transfer, partial-failure,
bulk stop/delete/all-page and prompt-during-bulk frames.
Inspect connection overview/arguments/matched-current-rule, draft creation,
truncation and existing-name frames 27–34 as described in the history feature.
The assertions require painted glyphs, not just a nonempty image file.
Frames 35–40 cover lasting popup options, live details, pending/observed catalogue
states, per-request reset and a temporary network match. Check the assertions
against actual gRPC replies as well as the images.
Frames 41–43 cover saved popup defaults, the resulting next-request choices and
disabled popups. The focused preferences run has its own thirteen-frame manifest;
do not treat it as full protocol or whole-application proof.
Frames 44–48 reject a zero-second timeout, save reported-field AND defaults and
Reject fallback, verify those fields in an actual RPC rule, show unavailable
fields for review, and let a one-second request expire to Reject once despite
its lasting selected duration. Core tests additionally change preferences while
requests wait, replace sessions and cap a 110-second override by a shorter RPC.

Foreground runs retain `doctor.json`, `instance.json`, `actions.json`,
`application.log`, `fixture.log`, `fixture-report.json`, screenshots, and
`result.json` under `.verification/native/<unique-run>/`. Require every cleanup
boolean to be true. Reports preserve binary identity and selected backend.

Headless input proves the actual GPUI views/controls and real SQLite behavior
with synthetic data at the backend boundary. Transport tests separately prove
gRPC semantics. Native tests combine the real desktop, Python client transport,
and database. None proves enforcement by the system OpenSnitch daemon, native
Wayland activation on other compositors, or lower RSS than Qt. State those
boundaries. Build success alone does not prove user interaction.

## Cleanup

The background wrapper waits for its child, terminates only that child process
group on timeout, and keeps evidence. The passing flow drops entity handles
before the GPUI context, joins the backend/history workers, and verifies removal
of its socket. Temporary fixture state is scoped to that test run.

The foreground helper stops only PIDs it launched, closes its own virtual input
daemon, removes its temporary socket directory, and keeps the history database
and evidence. It does not delete the shared advisory lock file. Never kill by
process name, unlink `/tmp/osui.sock`, or stop another input daemon. On an
interrupted run, inspect `instance.json` and `input-instance.json`, corroborate
PID/executable/socket identity, then clean up only those owned resources.

Both wrappers refuse an existing explicit `--evidence` directory. Evidence is
ignored by Git and retained after cleanup; it is not automatically pruned yet.

## Helpers

- `python scripts/verify-headless.py [--evidence /absolute/new/path]`: background GPU/input flow.
- `python scripts/verify-headless.py --feature countdown`: focused pointer/keyboard countdown review, real RPC and persistence proof.
- `python scripts/verify-headless.py --feature checksums`: focused checksum review, acknowledged mutations, failure/stop/all-page and permission isolation proof.
- `python scripts/verify-headless.py --feature history-tools`: typed filters, saved table columns, actual header sorts, CSV/JSON/clipboard data and sorted paging proof.
- `python scripts/verify-native.py doctor`: read-only native prerequisites.
- `python scripts/verify-headless.py --feature daemon-preferences`: typed daemon settings/review, ACK/failure/persistence and prompt isolation.
- `python scripts/verify-native.py run --feature history|history-tools|statistics|nodes|daemon-preferences|prompts|placement|countdown|checksums`: owned foreground proof/cleanup; placement requires explicit X11.
- `python -m unittest discover -s tests -p test_native_desktop.py`: native helper guards against Unicode lookup misses and stale-window reuse.
- `python scripts/fake-daemon.py --help`: synthetic daemon protocol fixture.
- `python scripts/cargo.py test --no-default-features --locked`: independent protocol/storage checks.

Use `/maintain-verification-skill` after app changes to keep entry points,
coverage, scripts, and proof boundaries current.

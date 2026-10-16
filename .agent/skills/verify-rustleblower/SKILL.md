---
name: verify-rustleblower
description: Verify Rustleblower's native GPUI permission prompts, history, daemon protocol, and resource bounds. Use real WGPU screenshots and GPUI input in the background, or FerryDock-derived owned Wayland/X11 foreground controls. Preserve evidence and scoped cleanup.
---

# Verify Rustleblower

The upstream reference is now the retained bare Git repository at
`/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch`, pinned to
`a1353848ba1b660320e90cefea782c3fba272c00` (source version 1.9.0). Read
[reference/delta scope](../../../docs/upstream-reference.md); update the reference
explicitly rather than following master. Historical 1.7.2 reports remain scoped
evidence. Proto identity does not prove new command/operator semantics. List
subscriptions are GUI-side local-node downloads, separate from daemon downloader
tasks; tests must use owned HTTP/socket/database fixtures.

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

Use `python scripts/verify-headless.py --feature tray` for the four-frame
[window/tray lifecycle](features/connection.md) journey: retained dashboard,
permission input while hidden, repeated Open, tray-host loss and dashboard Quit.
Callbacks are synthetic here. Use `python scripts/verify-native.py run --feature tray`
for owned compositor close, actual SNI/dbusmenu callbacks, hidden-dashboard gRPC
permission input and clean tray Quit. Neither proves a panel pointer click.
The native run also requires exact 16/22/24/32 px colour ARGB payloads matching
the approved tiny/compact asset manifest, recorded in `tray-icons.json`.

Use `python scripts/verify-headless.py --feature permission-actions` for six
frames covering whole-app and exact-hostname/IP scopes, direct lasting Allow
buttons, keyboard radio selection, preserved custom rules and unknown executable
handling. It checks exact gRPC predicates (no hidden port or wildcard), clearing
of saved constraints only after an explicit simple-scope choice, unchanged saved
preferences, six persisted decisions and owned cleanup.

Use `python scripts/verify-headless.py --feature permission-icons` for four
frames proving desktop-entry PNG/SVG lookup and painted Kit Avatar pixels,
initials when unavailable, and no stale icon on the next request. It holds both
icon workers while answering a real AskRule RPC to prove lookup cannot gate
permission replies. Its XDG fixtures live only in the owned evidence directory.

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

Use `python scripts/verify-headless.py --feature history-tools` for the sixteen-frame
[history conditions/table/export](features/history.md) journey. It combines UID
and command conditions, rejects an invalid port, saves/reopens selected columns,
sorts through the actual Kit header, exports all matches to CSV/JSON Lines,
checks existing-file preservation, copies a matching page and pages sorted results.

Use `python scripts/verify-headless.py --feature statistics` for the ten-frame
[statistics](features/statistics.md) journey: all categories, actual chart and
selectors, frozen paging, CSV/JSON/clipboard, exact history drilldown and confirmed
selected/category resets through the production gRPC counter path.

Use `python scripts/verify-headless.py --feature nodes` for the seventeen-frame
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

Use `python scripts/verify-headless.py --feature alerts` for the ten-frame
[daemon alerts](features/alerts.md) journey. It covers frozen paging, filtering,
typed details/clipboard, read/delete, CSV/JSON export, replacement/cancel and an
independent permission. `python scripts/verify-native.py run --feature alerts`
adds owned native opening/search and six exact persisted synthetic records.

Use `python scripts/verify-headless.py --feature desktop-notifications` for the
four-frame saved-toggle/hidden-dashboard/restoration journey. Run
`python scripts/cargo.py test --lib app::desktop_notifications::tests --locked`
for actual protocol and failure handling on a private owned D-Bus session.
Read [desktop notifications](features/desktop-notifications.md) for exact bounds
and the distinction between service acceptance and painted desktop delivery.

Use `python scripts/verify-headless.py --feature live` for the eleven-frame
[live inspection](features/live.md) journey. It verifies task start/stop JSON,
streamed snapshots, socket filters/paging, process fields, errors/resources and
independent permission input. After a desktop rebuild, `python scripts/verify-native.py
run --feature live` proves native opening/Start/search/Pause with an owned Python
fixture. Task stops have no success ACK in the pinned daemon; require the command
and honest Stop requested feedback rather than an invented confirmation.

Use `python scripts/verify-headless.py --feature history-preferences` for the
ten-frame retention/refresh journey. Read
[history preferences](features/history-preferences.md) for transactional save,
rollback, real idle-timer and CLI/reopen proof. The core suite includes a real
one-minute quiet wait. Native retention editing uses `python scripts/verify-native.py
run --feature history-preferences` after a desktop rebuild. Keep current tools
focus order and require exact saved settings, zero policy commands and cleanup.

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
forever by click → Deny once with Tab/Space. It checks resulting page contents and
the backend oneshot replies. Stable Kit IDs are `older`, `latest`, `all`,
`allow`, `deny`, `allow-forever`, `allow-12h`, `allow-selected`, and `deny-once`.
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
permission window, traverses the two scope radios and sends Space on Block
for deny/once (three Tabs), or Allow forever for allow/always (five Tabs)
with `--decision allow`. The legacy placement fixture has an extra Custom radio
and requires one additional Tab. Require the synthetic daemon reply
before the configured 30-second fallback could fire.

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
- `python scripts/verify-headless.py --feature permission-actions`: direct lasting Allow buttons, real RPC durations and persistence.
- `python scripts/verify-headless.py --feature countdown`: focused pointer/keyboard countdown review, real RPC and persistence proof.
- `python scripts/verify-headless.py --feature checksums`: focused checksum review, acknowledged mutations, failure/stop/all-page and permission isolation proof.
- `python scripts/verify-headless.py --feature history-tools`: typed filters, saved table columns, actual header sorts, CSV/JSON/clipboard data and sorted paging proof.
- `python scripts/verify-native.py doctor`: read-only native prerequisites.
- `python scripts/verify-headless.py --feature daemon-preferences`: typed daemon settings/review, ACK/failure/persistence and prompt isolation.
- `python scripts/verify-native.py run --feature tray|history|history-tools|statistics|nodes|daemon-preferences|prompts|placement|countdown|checksums`: owned foreground proof/cleanup; placement requires explicit X11.
- `python -m unittest discover -s tests -p test_native_desktop.py`: native helper guards against Unicode lookup misses and stale-window reuse.
- `python scripts/fake-daemon.py --help`: synthetic daemon protocol fixture.
- `python scripts/cargo.py test --no-default-features --locked`: independent protocol/storage checks.

Use `/maintain-verification-skill` after app changes to keep entry points,
coverage, scripts, and proof boundaries current.

Use `python scripts/verify-headless.py --feature firewall` for the eight-frame
[partial system-firewall journey](features/firewall.md): actual chain browsing,
paging/search, keyboard complete-JSON edit, review/back/confirm, held ACK with
independent permission input, exact protobuf/cache provenance and rejected draft.
The separate field-form and file/profile journeys below cover additional paths;
this journey alone does not prove them or enable/disable. Never connect these fixtures to the user's daemon.

Use `python scripts/verify-headless.py --feature history-process` for the eight-frame
[history-to-current-process journey](features/live.md#history-to-process-inspection).
It verifies event-node targeting with two gRPC fixtures, keyboard activation, PID
validity/disconnect guards, queued-session replacement refusal, current-data copy,
error retention and independent permissions. Native shortcut input is separate.

Use `python scripts/verify-headless.py --feature firewall-forms` for the thirteen-frame
[chain/rule/expression/value journey](features/firewall.md#native-field-editors).
The wrapper selects the exact Rust test; prefix selection would run both firewall
journeys into one evidence directory. After rebuilding desktop,
`python scripts/verify-native.py run --feature firewall-forms` adds owned native
Wayland chain-input/review/ACK/persistence proof. It uses the explicit in-memory
`--allow-system-firewall` fixture; no host firewall command is executed.

Use `python scripts/verify-headless.py --feature firewall-files` for the eleven-frame
[file/profile interchange journey](features/firewall.md#files-and-imported-profiles).
Require two profile-list pages, typed file paths, bounded import/merge/removal,
exact full/profile exports without overwrite, reviewed complete protobuf, held ACK
with independent permission, accepted cache and offline export. Native OS picker
interaction is not covered by this background journey.

Use `python scripts/verify-headless.py --feature firewall-policy` for the
[eleven-frame policy/state sequence](features/firewall.md#built-in-policies-and-reviewed-state-transitions).
Require preset/checkbox and shortcut input, explicit complete step review, held
ACK and independent permission, exact policy preparation then Disabled, detached
editor-close completion, reopening/enabling and rejected-draft preservation.
After desktop rebuild, `python scripts/verify-native.py run --feature firewall-policy`
adds owned native shortcut/two-stage ACK/persistence proof. Never use the real
daemon: `--system-firewall-drop` is an in-memory synthetic fixture flag only.

The full goal also requires the final [UI consistency pass](../../../docs/ui-consistency-plan.md)
after feature implementation. Verify shared layouts, full-matching-dataset sorts,
semantic colors, predictable editing/context retention and whole-app visual
consistency using actual screens and input. Subscription setup must open a usable
empty screen from obvious navigation and create its config through UI actions;
manual JSON bootstrap is not an acceptable substitute.


Use `python scripts/verify-headless.py --feature subscriptions` for the
[blocklist catalogue/download workflow](features/subscriptions.md). It wraps the
GPU test in a guarded owned network namespace with only loopback; require
`network.json` with different parent/current namespaces in addition to normal
GPU/socket cleanup evidence. The complete core suite now includes actual HTTP
fixtures: run it through `scripts/owned_net.py` as documented in the feature map.
After a desktop rebuild, `python scripts/verify-native.py run --feature subscriptions`
proves actual empty-screen navigation/name/URL input/save and durable metadata,
with automatic updates paused and zero HTTP or policy commands. Preserve the
separation between downloads, canonical daemon list matching and associated rules.


The subscription journey now includes fifty-six frames covering canonical data and
single/group/all collections and associated rules: reviewed/cancelled creation, exact held ACK,
independent permission and confirmed-rule second-page editing/rejection. The shared
editor displays its target node; rerun the general journey after changing it.
`scripts/verify-list-matching.py --evidence /absolute/new/path` adds isolated Go
reader/matcher/monitor proof against the retained exact source pin, using actual
production-normalized HTTP fixture files. Read the subscription feature map for
Go/source requirements and the distinction from daemon/kernel enforcement.


`python scripts/verify-native.py run --feature subscription-collections` proves
native empty collections, UID-scoped all-domains review, full long-directory
inspection via Tab/End, keyboard Cancel and restored Ctrl+F filtering. Require
four captures, zero policy commands/subscription bootstrap, 430 history rows and
all cleanup flags. Window-manager close minimizes the existing generic editor;
it is not Cancel. Use its actual Cancel action. This is native review/navigation
proof; native saves, membership application and paging remain separate. The
56-frame GPU journey covers collection query/sorts, membership review/input and
bulk configuration across pages with stale/error refusal and explicit overrides.


`python scripts/verify-native.py run --feature subscription-bulk` adds owned native
metadata setup, status Select filtering/selection, exact before/after review and
keyboard application. It retains eight PNGs and requires the single expected
persisted configuration with enabled=false, checked=0, no downloaded file/bytes,
paused updates, zero policy commands, 430 history rows and all cleanup flags.
Wait for rendered frames after Select filtering before confirming. The 56-frame
subscription GPU flow separately covers marks across pages, group modes/limits,
whole-filter scope, Back/Cancel, stale refusal/rebuild and checked limits. Read the
feature map for the transaction and filesystem recovery boundaries.


`python scripts/verify-native.py run --feature subscription-transfer` proves real
Ctrl+Shift+T file-path input, OpenSnitch legacy/group/unit review and confirmed
keyboard import. Require three PNGs, the exact persisted native-test/120-second/
2-KiB configuration, unchanged defaults, zero checked/downloaded bytes/files and
policy commands, 430 history rows and complete owned cleanup. Ctrl+Shift+I opens
GPUI's inspector, so it is deliberately not an import shortcut.

For independent pinned codec proof, set `RUSTLEBLOWER_INTERCHANGE_EVIDENCE` to a
fresh absolute evidence directory while running the subscription_transfer core
test. It records create-new-only Rust decoder/worker fixture JSON. Then run
`python scripts/verify-subscription-interchange.py --fixtures /absolute/fixture/path --evidence /absolute/new/codec-proof`.
The helper reads exact-pin source from the retained bare Git repo, loads only pure
models/helpers with an owned XDG stub and compares nine normalization cases plus
a production action export. It runs no plugin, Qt, downloader or daemon runtime.
See the feature map for duplicate-name, unit-only inheritance, notification,
resource caps and filesystem recovery boundaries.


`python scripts/verify-native.py run --feature subscription-catalogue` proves an
owned old subscription, complete replacement with two same-name identities,
reviewed retained removal and keyboard apply. Require three PNGs, two active and
one retained-removed row, distinct source extra fields bound to IDs 2 and 3,
unchanged defaults, zero checks/downloaded files/bytes and policy commands,
430 history rows and complete owned cleanup. This does not prove native archive
inspection or retained-file restoration. The 56-frame background subscription
journey covers real rename/duplicate/export plus 84-entry retained-removal review,
paging/cancel/apply and restore with exact extra fields/data. The eight transfer
core cases cover one-time legacy migration/archive/reimport, identical native
identities, source-compatible filename exports, late removal rollback and
6-MiB aggregate review rejection. Pinned Python codec proof now has 12 checks.
Read the current subscription feature map for bounds and remaining gaps.


Use `python scripts/verify-headless.py --feature port-ranges` for the eight-frame
[inclusive source/destination range journey](features/rules.md#inclusive-port-ranges).
Require real Select/Input/scroll/JSON/save/cancel/clipboard events, no command on
invalid input or unconfirmed daemon capability, exact original range wire text,
held ACK with independent permission, durable success and rejected-draft
preservation. Three dedicated core cases and `scripts/verify-port-ranges.py`
compare 21 actual Rust parser fixtures to unmodified pinned Go bodies. Read the
feature's 1.8/backport and native/enforcement proof boundaries. Full core now has
140 tests; rebuild the desktop after no-default-features tests.


The Nodes journey now adds opt-in per-node saved pause, current-generation
reconnect restoration under held ACK, independent permission, opt-out without
resuming and explicit manual resume. Its 17 frames complement three interception
transport/persistence cases and one cache-bound/corruption unit case (140 total
core tests). A real held SQLite write transaction after ACK must keep restoration
busy while Ping/AskRule remain independent. Read the Nodes feature for source
scope, failure feedback and bounds. `python scripts/verify-native.py run --feature interception-preference`
proves native checkbox input and durable local preference with zero daemon
commands. Recheck `--feature daemon-preferences`: node search now needs four Tabs
to its configuration button. Neither proves native interception enforcement.


Use `python scripts/verify-headless.py --feature cross-node` for the 14-frame
[reviewed cross-node copy journey](features/rule-distribution.md). Require actual
cross-page marks, selected/full-filter scope, target checkboxes, source JSON,
ACK/permission/Stop/results/rebuild/replacement input and owned-window destruction
under held ACK with no unsent target mutation. Three core cases cover 230-rule
snapshots, conflicts/observation exclusions, partial node errors, command
reservations, actual checkpoint during RPC wait, stale sessions/configurations,
range/capacity and source-allocation guards. Full core now has 143 tests. This
historical slice did not complete direct multi-node draft/rename or file interchange,
native compositor copy controls or kernel enforcement. Rebuild desktop after
core tests; preserve the existing socket and all owned evidence.


Use `python scripts/verify-headless.py --feature multi-node-drafts` for the
14-frame [unsaved draft, rename and import journey](features/rule-distribution.md).
Require staged editor input and Cancel preservation, explicit destination review,
new-name ACK before old-name deletion, held ACK with independent permission,
Stop/continue at the removal stage, durable two-node results, typed file and
captured-clipboard imports with immutable reviewed contents, and invalid
clipboard refusal without silent file fallback. Six cross-node core cases cover
shared allocation, partial removal failure and deletion-only fresh-review
recovery in addition to the existing copy guards. Full core now has 146 tests;
run the old cross-node and general GPU journeys after shared-engine changes.
Rebuild the desktop after core tests. Multi-node export, new native OS picker/
compositor proof and real kernel enforcement remain separate.


Use `python scripts/verify-headless.py --feature multi-node-export` for the
eight-frame [multi-node folder export](features/rule-distribution.md#multi-node-folder-export)
journey. Require cached/offline selection, typed parent path, full/filter count
review, Cancel with no output, frozen contents after inventory replacement,
independent permission during review, failed destination preservation and exact
per-node JSON outputs after retry. Four export integration tests and one unit
case add bounded-source, actual checkpoint, identity and late-stage rollback
proof. The complete current core suite has 151 passing cases. Native OS picker and real-daemon import/enforcement remain unproved.
Rebuild desktop after any no-default-features run.


Use `python scripts/verify-headless.py --feature startup` for the six-frame
[startup preferences](features/connection.md#startup-preferences) journey. It
sets and asserts owned XDG configuration roots; require actual staged/Save/Close/
Reload, durable enable/disable/reopen, synthetic tray Startup routing, real
permission during save and stale-file preservation. Three core cases and
`scripts/verify-startup-entry.py` separately prove file precedence/preservation
and actual desktop-entry/GIO argument decoding of a fixed owned recorder.
Real login scheduler, new native tray-menu callback/compositor behavior and
packaged AppImage execution remain separate. Recheck the general and tray GPU
flows after navigation changes; rebuild desktop after core tests.

The startup slice records 154 passing core cases, three final focused path/file
cases, strict Clippy and six startup plus 48 general/four tray GPU frames.


Use `python scripts/verify-headless.py --feature appearance` for the eleven-frame
[native color appearance](features/appearance.md) journey. Require actual Kit
Settings mode/reset/reload input, existing dashboard/editor/popup repaint, copied
unsaved fields with no saved rule, real independent AskRule, stale-write refusal
with session-only color, explicit invalid-file recovery, fresh GPUI-context saved
mode and reset. Three core appearance cases and the history-export protection
case cover bounded files and failure guards. The platform is fixed Light: real
native system appearance-change callbacks, custom themes/density and the final
whole-app visual pass are not claimed. Run general/tray regressions after shared
composition/navigation changes and rebuild desktop after core tests. The current
appearance slice records 157 passing core cases plus the final history-export
protection case, strict Clippy and eleven/48/four GPU frames.


Use `python scripts/verify-headless.py --feature window-state` for the eleven-frame
[main-window persistence](features/connection.md#remembered-main-window) journey.
Require real navigation/Quit/permission input, exact warm/cold saved state,
coalesced geometry updates, held load-result timeout without request gating,
stale-file preservation and pending-request quit fallback plus layout flush.
Three core cases and the reserved export filename bring full core to 160 cases.
After desktop rebuild, the owned KDE `--feature window-state` then `--feature
window-restore --restore-state /absolute/prior-passed-evidence/history.sqlite3.window.json`
prove real resize/maximize/fresh-process last-page restore and native normal-size
recovery. Validate prior report/SHA, fractional geometry, zero policy commands,
430 rows and complete scoped cleanup. Native fullscreen/X11/disconnect/focus
variants and package/RSS remain separate. Rerun general/tray GPU after lifecycle
changes; retain all failed harness attempts.

History-tools now retains sixteen frames. The added six prove actual search/action/
condition/header/width input, immediate Quit save, cold backend/GPUI restoration
without old paging cursors, stale-file preservation with session-only feedback,
independent real AskRule/Deny once and explicit fixture repair/restart. Four focused
core cases bring full core to 164 and include corrupt-file RPC startup fallback.
Version-3 settings cap all presentation state at 16 KiB, migrate versions 1/2 and
merge column/refresh/presentation writes. Read the History feature's exact proof
and recovery boundaries. Recheck general/tray/window-state after Quit changes.

Use `python scripts/verify-headless.py --feature subscription-url` for eleven
[pre-save URL test](features/subscriptions.md#test-a-url-before-saving) frames.
The wrapper requires its own loopback namespace. Require actual Test/Cancel/format/
URL/form/navigation/Save input, HEAD/GET/redirect/hosts sample assertions, stale
result discard, an independent AskRule during held GET and no metadata/data write
before explicit Save. Six core probe cases bring full core to 170; a nine-case
pinned Python pure-helper comparison independently checks the hosts heuristic.
The unchanged 56-frame subscription journey and general/tray flows cover shared
forms, worker commands and lifecycle. Native `--feature subscription-url` verifies
the fixed footer by keyboard and rejects an unsupported URL scheme before I/O;
require three PNGs, OCR, zero subscriptions/logs/commands, 430 events and scoped
cleanup. Native successful HTTP/cancel, certificate/compression/resource/package
proof and the final whole-app UX remain separate. Read the feature's exact caps,
short-sample limitation and archive-recovery-versus-source-parity distinction.

Use `python scripts/verify-headless.py --feature subscription-inheritance` for
twelve [independent count/unit](features/subscriptions.md#independent-countunit-inheritance)
frames. Require actual individual/default unit input, empty-count inheritance,
changed default counts and units, count-only overrides, dependent-limit refusal,
stale-default save refusal, fresh reopen, legacy absolute preservation and real
permission input. Four core cases bring full core to 174, including atomic SQL/
log/schedule rollback and an actual inherited HTTP size cap. Compare fifteen
effective-limit tuples against the pinned pure Python models using the existing
interchange script's `--inheritance-cases` argument. Native
`--feature subscription-inheritance` must persist null counts and the selected
minutes/seconds/KB units with no download/policy command, 430 events and complete
cleanup. Recheck native URL validation and source import because three selectors
change keyboard navigation. Keep the 56/11/48/four subscription/URL/general/tray
GPU regressions and rebuild desktop after core tests. Independent bulk unit
controls, native default/stale/reopen variants, real schedule timing, package/RSS
and final UX remain separate; do not claim them from this slice.

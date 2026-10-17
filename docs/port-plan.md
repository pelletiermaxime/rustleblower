# OpenSnitch UI parity plan


Current upstream target: **OpenSnitch master pinned at a1353848ba1b660320e90cefea782c3fba272c00**
(source version 1.9.0; installed package label 1.8.0.r224). See
[reference and delta audit](upstream-reference.md). Earlier 1.7.2 evidence below
remains historical. Native subscriptions, source/destination port ranges and
opt-in persisted pause now have verified implementations. Next parity slices
cover remaining subscription runtime gaps and cross-node rule operations; the
full scope is retained.
A [required final UI/UX pass](ui-consistency-plan.md) follows feature implementation:
shared page patterns, meaningful global table sorting, semantic colors and
whole-app rendered/input proof are part of completion, not optional follow-up.

The project objective is full native UI parity while keeping the existing daemon
and its policy ownership. The saved full-port goal has resumed after the daemon-preferences stopping point;
continue through the remaining inventory while preserving concurrent work. A working screen or a successful build alone does not
complete a slice: record protocol, persistence, background GPU/input, and native
proof separately in the verification map. Keep unproven work marked pending.

## References and scope

- Installed OpenSnitch UI/daemon: 1.7.2. UI sources are under
  `/usr/lib/python3.14/site-packages/opensnitch`.
- Latest published upstream release checked October 15, 2026:
  [v1.8.0](https://github.com/evilsocket/opensnitch/releases/tag/v1.8.0).
  Compare its UI/protocol after implementing the installed daemon's features;
  expose version-dependent capabilities accurately.
- [Protocol](https://github.com/evilsocket/opensnitch/blob/v1.7.2/proto/ui.proto),
  [notification handlers](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/ui/notifications.go),
  and installed `dialogs/stats.py`, `dialogs/ruleseditor.py`, `dialogs/preferences.py`,
  `dialogs/firewall*.py`, `dialogs/conndetails.py`, `dialogs/processdetails.py`,
  and `res/*.ui` define parity.
- FerryDock is the reference for feature modules, pinned Kit components, bounded
  asynchronous state, native integration, and reproducible verification.
- Preserve bounded payloads, disk-backed catalogues/history, virtualized pages,
  and prompt isolation. Tests use owned sockets, databases and daemon fixtures.
  Do not change the real daemon or firewall to verify a feature.
- Use native equivalents for Qt platform-specific preferences. Keep supported
  upstream behaviors visible; do not call an omitted feature complete.

## Feature inventory

| Slice | User-visible behavior | Status |
|---|---|---|
| Local connection | UI-side gRPC, connection status, reconnect, socket ownership | Implemented and verified on isolated protocol/GPU/client fixtures; user also confirmed real traffic |
| History | Bounded persistent events, text/action filters, paging, accurate loss reporting | Persistent summaries/details, typed AND filters, Reject action, configurable Kit table, global keyset sorting and CSV/JSON/clipboard export implemented. Core, focused/full GPU and native UID-filter proof recorded below; native picker/export/resize controls and further history preferences remain pending |
| Prompt basics | Separate window, allow/deny once, deadline, disconnect/cancellation/overload | Implemented and verified |
| Application rules (first priority) | Browse/filter permanent/temporary rules; create, edit, allow/deny/reject, enable/disable, delete | Implemented for the connected node, including full editor. Protocol/persistence/background GPU verified; native input proof pending |
| Rule editor | All application/network predicates, regex/list/CIDR/checksum operators, priority/logging, all durations | Implemented with Kit inputs/selects/checkboxes and match JSON. Core validation and GPU rename/metadata/regex/duration/JSON roundtrip verified; every field variant and native compositor input remain to verify |
| Rule workflows | Duplicate, multi-select/bulk actions, import/export JSON and clipboard, apply to nodes, create from event | Duplicate, cross-page selection, bulk actions, file/folder/clipboard interchange, and reviewed application/destination drafts from history implemented for current node. Core/background GPU verified; per-node catalogue/editor targeting, reviewed cross-node copies, unsaved drafts/rename and file/directory/clipboard imports are available; reviewed all/filter multi-node folder export is implemented; broader native picker/compositor proof pending |
| Prompt options | Reject, duration/target choice, advanced multi-field matching, defaults and position, disable popups | Reject, nine duration presets, derived command/domain/network/checksum targets, AND constraints, priority/logging, live details, saved basic/advanced defaults, UI timeout/fallback controls and disabling popups implemented; core/background GPU verified. Saved X11 initial placement has native geometry/input proof for Center/four corners; native Wayland retains compositor placement with the selector disabled. Queued requests retain captured settings; unavailable default fields open review feedback. Lasting replies await daemon observation; explicit inventory refresh available. Interaction pauses the ordinary countdown with a visible finite RPC limit; pointer/keyboard GPU and Wayland/X11 input proof cover it. Checksum-change warnings and a separate selected/all update review are implemented for enabled exact executable/hash AND rules, with core/GPU and native review-window input proof. Native update controls and broader input proof remain pending |
| Connection details | Source/destination, PID/UID, command, process tree, checksums, copy and rule creation | Historical metadata/arguments/matched-rule snapshot, copy, current-rule lookup and reviewed rule creation implemented with bounded disk snapshots. Current process/tree/checksums are available through the new event-node Live shortcut, with PID-reuse feedback. Core/GPU verified; UID-name resolution and native connection/shortcut input remain pending |
| Statistics | Hosts/apps/addresses/ports/users/protocols, counts, charts, drilldown filters, reset/export | Implemented with bounded persistent daemon-counter cache, node/search/sort controls, Kit top-ten chart, frozen paging, exact history drilldown, local-baseline reset and CSV/JSON/clipboard export. Core/GPU and native Wayland/X11 open/search/persistence verified; native reset/chart/export/pickers and long-duration RSS remain pending |
| Nodes | Multiple nodes, detail/resources, interception controls, per-node configuration and rule targeting | Independent named Unix sessions, bounded cached metadata, per-node rules, confirmed interception/configuration, scoped refresh and live Sysinfo resources through Live implemented. Core/GPU and native read/search/persistence verified; peer identity/TCP/TLS, node removal/export and broader multi-node native proof remain pending |
| Daemon preferences | Default action/duration, logging, monitors, rule paths, network/device options | 29 typed fields in five groups, validated changed-field patching, bounded before/after review and complete JSON implemented. Unknown/unreported values and exact decimal numbers preserved. Core, focused/full GPU and native default-action edit/review/save verified; apply-to-all-nodes, exhaustive field variants and actual TLS connection proof remain pending |
| Remote transports | TCP, TLS/mutual TLS, certificates, connection/auth configuration | Pending; revisit local-only authority compatibility exception before TCP |
| System firewall | Profiles, chains, ordered rules, full expression editor, enable/disable and reload acknowledgements | Partial: bounded model/interchange, guarded complete reload and accepted/report provenance; dedicated chain/rule CRUD, enabled/order/duplicate, expression/value editors and common staged review implemented. Core/GPU and native chain-input proof recorded below. File/imported-profile interchange and bounded on-disk profile listing now share the same draft/review. Built-in selected-chain policy presets and acknowledged enable/disable sequencing are now implemented; condition presets/validation and broader native controls remain unfinished; see [full scope](system-firewall-plan.md) |
| Live sockets/processes | Netstat filters and polling; daemon tasks for sockets/process details; stop task lifecycle | Live tab, daemon filters, bounded snapshots/paging, PID inspection, complete field/raw copy, process error retention, node Sysinfo resources and history-to-current-process targeting implemented. Core and focused/full GPU proof recorded below; native Wayland opening/Start/search/Pause verified earlier. Native history shortcut/process-field/picker/paging/copy, exports, persisted filters/columns and real kernel visibility remain pending |
| Alerts | Persist/display/filter/copy/export text and typed alerts, rule alert configuration | Bounded schema-7 archive, all five payload types, session attribution, node/severity/text/unread filters, frozen paging, detail/clipboard, read/deletion and full CSV/JSON export implemented. Core, focused/full GPU and native Wayland opening/search/persistence verified. Desktop notification worker/urgency/toggle/Open action implemented and verified on private D-Bus and background GPU fixtures. Per-rule settings and exhaustive native mutation controls remain pending |
| Desktop | Tray, desktop notifications, autostart, window restoration, appearance/density/language/shortcuts | Close-to-tray and retained dashboard lifecycle implemented. Daemon-alert desktop notices, urgency, saved toggle and Open alerts restoration implemented and verified on private-bus and GPU fixtures. Staged per-user XDG autostart is implemented with main/tray preference access, AppImage-aware paths, stale/foreign-file preservation and independent permissions; native System/Light/Dark color preferences are implemented with live repaint and bounded asynchronous persistence; main-window geometry/mode/last-page persistence is implemented with bounded background startup/write/quit handling and owned KDE maximize/restart/unmaximize proof; custom themes/density/language, other layout/filter state, permission notification alternatives and broader shortcuts remain pending |
| History preferences | Retention/database settings, event columns, temporary-rule filters, refresh intervals | Compact/table mode, columns and manual/automatic display refresh persist in a bounded document. Reviewed transactional retention/disk-target/idle cleanup and CLI override persistence implemented; core/focused/regression proof recorded below. Native Wayland retention review/save is verified. Database relocation/backend/journal choices, temporary-rule filters and persisted widths/sort/filter presets remain pending |
| Plugins and extensibility | Inventory upstream plugin features and supply native equivalents/integration where applicable | Pending |
| 1.8 compatibility | Compare latest UI and wire protocol; add supported features and capability feedback | Pending |
| Upstream ideas | Review issues/PRs after parity; document concrete value and implementation evidence | Pending |
| Little Snitch ideas | Review official product/docs, then adapt useful workflows to the existing daemon | Pending |

## Deferred enhancements

- [ ] **Per-application bandwidth without an OpenSnitch fork** (requested October 15, 2026).
  Show live upload/download rates and byte totals per application, with destination
  breakdown where attribution is reliable. Keep the existing OpenSnitch daemon
  responsible for firewall policy and permission decisions.

  Start with an isolated eBPF measurement prototype, then assess reusing an
  existing collector versus a small Rustleblower helper. Keep privileged
  collection separate from the unprivileged GPUI app. Collector I/O, ingestion,
  storage and failures must never delay permission handling; bound queues,
  payloads, retained history and UI pages. Missing collection should appear as
  unavailable statistics rather than zero traffic.

  Before implementation, verify TCP and UDP/QUIC, IPv4/IPv6, short-lived processes
  and connections, PID reuse, containers/network namespaces, required privileges,
  kernel compatibility, overhead and cleanup. Define whether counters represent
  application bytes or network bytes including retransmissions. Label initial
  totals as "since monitoring started"; persistent daily history is a later
  extension. Do not inspect payload contents or change firewall rules to collect
  bandwidth. Verification must use owned workloads and preserve the system daemon
  and its socket.

  References: [Picosnitch](https://github.com/elesiuta/picosnitch),
  [Netdata socket collector](https://learn.netdata.cloud/docs/collecting-metrics/collectors/operating-systems/ebpf-socket),
  and [OpenSnitch's experimental bandwidth work](https://github.com/evilsocket/opensnitch/issues/1081).
  This is a deferred TODO, not authorization to start implementation automatically.

## Execution notes

First slice evidence: `.verification/rules-core-tests.log`,
`.verification/rules-clippy.log`, and
`.verification/headless/20261015T061753Z-5fc52e1d/result.json` (12 screenshots).
The background flow caught and fixed the list's flex layout and focus restoration
after the new-rule form closes. No real daemon policy was changed for verification.

Full editor evidence: `.verification/editor-core.log` (24 core tests),
`.verification/editor-clippy.log`, `.verification/editor-build.log`, and
`.verification/headless/20261015T064401Z-a294f462/result.json` (17 screenshots).
The flow edits a rule through actual Kit input, renames with acknowledged
save/delete, adds a case-insensitive regex predicate, cancels and saves a
duplicate, roundtrips structured conditions through match JSON, and rejects an
invalid custom duration before accepting `90s`. The original duplicate source
retains its duration and operators. The installed 1.7.2 protocol serializes only
one AND level: nested groups are rejected on editing/import validation to avoid
silently discarding children. JSON import/export helpers exist, but their user
workflow is not yet implemented.

The notification stream already supports rule changes and deletion. Do not
optimistically report a rule as applied before the daemon's matching success
reply. Bound pending commands, handle failures/timeouts/disconnects, and reject
stale-session replies. Keep full rule operators intact when changing one field.
Store rule inventories on disk and query bounded pages instead of retaining all
rules in the UI. Track the completion and proof of each slice here and in the
project verification skill as work proceeds.

Rule transfer/bulk evidence: `.verification/transfer-core.log` (30 core tests),
`.verification/transfer-clippy.log`, `.verification/transfer-build.log`, and
`.verification/headless/20261015T070759Z-4cdb537b/result.json` (26 screenshots).
The flow verifies file/clipboard transfer, adjacent OpenSnitch JSON objects,
default duplicate skipping and explicit replacement, partial failure/retry,
cross-page selection/export of 201 names, stop/resume after a held reply,
cancelled/confirmed bulk deletion, and acknowledged disable of all 229 remaining
rules across pages. It answers a separate permission popup while the reply is
held. Core tests cover directory formats and a 260-rule consistent export while
another connection changes a later page. File pickers and native compositor
input remain unverified for these new flows; no real firewall policy was changed.

Source review of 1.7.2 `Loader.Replace`/`Loader.Delete` shows memory changes can
precede disk errors. An ERROR retains the last confirmed local cache but now sets
persistent uncertainty feedback until a fresh inventory arrives. Do not describe
an error as proof that live policy stayed unchanged. Retry is explicit and does
not force a reconnect that would interrupt pending permission requests.

Connection detail/history-rule evidence: `.verification/history-details-core.log`
(34 core tests), `.verification/history-details-clippy.log`,
`.verification/history-details-build.log`, and
`.verification/headless/20261015T073810Z-b071ca16/result.json` (34 captures).
The GPU flow copies real metadata/arguments, distinguishes historical from current
rule values, confirms a permanent executable+host/port/protocol draft, rejects
existing names in the full editor and quick form without commands, and displays
argument truncation. Snapshot encoding is capped at 24 KiB with incremental shared
queue reservation. Schema 2 preserves older summaries, cascades detail deletion,
and accounts for details in retention. The size test caught fixed-block eviction
emptying small histories; cleanup now removes only the needed oldest prefix.
Replay fingerprints sort protobuf maps; core tests verify persistence, UTF-8/
argument limits, IPv6 destination drafts, legacy migration, replay deduplication
and cleanup. New details have no native compositor proof yet. No real daemon,
socket or firewall policy was modified to run these checks.

Expanded permission evidence: `.verification/prompt-options-core.log` (41 core
tests), `.verification/prompt-options-clippy.log`,
`.verification/prompt-options-build.log`, and
`.verification/headless/20261015T082122Z-d53fb8d4/result.json` (40 captures).
Kit input selects a permanent Reject with command/UID/port/checksum conditions,
priority and no logging, scrolls lower constraints, and copies live details.
Actual gRPC replies and SQLite assertions distinguish an unacknowledged popup
proposal from a daemon-reported rule. A subsequent request resets options;
another allows a five-minute destination network. The core suite covers all
preset/action combinations, matching derived domains/networks/AppImage paths,
choice races, deadlines, excessive allocations and explicit inventory refresh
without replacing the owned socket. Schema 3 separates bounded popup proposals
from confirmed entries and migrates atomically. Native advanced-popup and real
daemon installation proof remain pending. No real daemon policy was changed.

Popup preferences evidence: `.verification/preferences-core-final.log` (44 core
tests), `.verification/preferences-clippy.log`, `.verification/preferences-build.log`,
and `.verification/headless/20261015T084000Z-ca88e895/result.json` (43 captures).
The focused `--feature popup-preferences` flow also passes at
`.verification/headless/20261015T084218Z-57c3f7b5/result.json` (three captures).
Saved defaults cover duration, target (including PID), priority and logging;
disabling popups answers new requests immediately once. The full flow verifies
the configured next-request gRPC reply and disabled-popup path. Requests capture
defaults on admission; a later settings save cannot change queued requests.
Core tests reopen the preferences file, reject bad/future/oversized files,
preserve memory after failed replacement and validate unavailable-target fallback.
The bounded document and background writer are independent of telemetry/SQLite.
GPU input exposed the pinned Select's full-height wrapper contract; explicit
32-pixel parent containers now keep its painted control and hit target aligned.
The preferences form scrolls in smaller windows. Native preferences input,
saved advanced constraints, popup placement, UI fallback/timeout preferences and
interaction-driven countdown pausing remained pending at that stage. The next
slice below implements the saved conditions and timeout/fallback controls.

Advanced preference evidence: `.verification/advanced-preferences-core.log`
(48 core tests), `.verification/advanced-preferences-clippy.log`,
`.verification/advanced-preferences-build.log`,
`.verification/headless/20261015T085529Z-497d6e2f/result.json` (48 captures), and
`.verification/headless/20261015T085755Z-e4fa6065/result.json` (five focused captures).
The installed `preferences.py` and `dialogs/prompt/__init__.py` define the four
saved fields: destination IP/port, UID and checksum, plus initial advanced display.
Rustleblower resolves only available values from each request and opens a warning
when a selected field is unavailable. GPU input verifies five actual returned
operators, missing-field feedback, invalid-zero preservation, a saved five-second
Reject default and an actual one-second expiry to Reject once while a lasting
duration is selected. Timeout/fallback overrides and conditions are captured on
admission. Core tests change preferences while a request waits, replace sessions,
shut down and cap a 110-second setting by the daemon's shorter RPC deadline.
That slice wrote version 2, read version 1 with defaults and rejected unknown versions;
older binaries reject the new format instead of ignoring overrides. Numeric input
retains at most three ASCII digits, conditions are limited to four known keys,
and no automatic fallback requests a lasting rule. Popup placement, interaction
countdown pause, checksum-change warnings and native input proof remained pending
at that stage; the placement slice below adds native geometry/input proof.

Placement evidence: `.verification/placement-core.log` (50 core tests),
`.verification/placement-clippy.log`, `.verification/placement-build.log`,
`.verification/headless/20261015T094633Z-8c7db438/result.json` (48-frame full flow)
and `.verification/headless/20261015T101851Z-6b9c0dfc/result.json` (13 focused frames).
The focused journey saves a position, checks all five production window bounds,
uses Tab/Space, and replaces a queued corner popup with a centered one. Geometry
unit tests cover negative origins and small workspaces; preference tests migrate
versions 1–2, write version 3 and reject unknown placement values. Runtime desktop
I/O uses one cached rectangle and a one-slot move channel on the background pool.

Native KWin/XWayland geometry and input passed consecutively for all positions:
center `.verification/native/20261015T100410Z-da98e0ef/result.json`, top-right
`.verification/native/20261015T100424Z-bf834ca1/result.json`, bottom-right
`.verification/native/20261015T100435Z-4f8ea107/result.json`, top-left
`.verification/native/20261015T100448Z-23e94cc1/result.json`, and bottom-left
`.verification/native/20261015T100501Z-ef7f3802/result.json`. Each report requires
actual outer-frame geometry within two logical pixels (minimum three physical),
a real input reply before fallback, persisted decision history and full cleanup.
The verifier reads positions but never moves a window. Native Wayland history,
Deny and Allow also passed on the same final desktop binary in
`20261015T101458Z-28fc5169`, `20261015T101508Z-2d482b0c`, and
`20261015T101519Z-9a469555` under the native evidence
directory; these prove input/persistence, not Wayland corner positioning.

Native checks exposed UTF-8/legacy X11 title lookup and stale-window reuse in the
helper, pre-focus input timing, native frame extents, and the need to acknowledge
a move before closing its temporary connection. The helper now clears old
handles, filters owned PID matches by UTF-8 name, waits for focus and records
rendered keyboard-focus frames. Two helper guard regressions prevent the title
and stale-handle failures from producing false proof. Native failure evidence is
retained. Placement is only an initial request: user movement/expansion does not
re-anchor it, and native Wayland displays its compositor limitation explicitly.


Countdown-review evidence: `.verification/countdown-core-final.log` (52 core
checks), `.verification/countdown-clippy-final.log`,
`.verification/countdown-build.log`,
`.verification/headless/20261015T102626Z-be3bb5e4/result.json` (four focused frames)
and `.verification/headless/20261015T102728Z-eba10cab/result.json` (48 full frames).
Installed 1.7.2 `PromptDialog._stop_countdown` stops its normal countdown after
interaction; pinned `daemon/ui/client.go` uses a 120-second AskRule context.
Rustleblower captures a finite hard limit at admission and shows its remaining
wait during review. Pointer press, keyboard input and initially displayed advanced
options pause only the ordinary timer for that request. Other queued requests
remain independent. One watch value per pending RPC updates expiry without UI,
telemetry, SQLite or per-input tasks. Repeated input never renews the hard limit.

Core proof covers review surviving the ordinary timer, another queued request
expiring, a shorter 1.8-second RPC limit with its captured Reject once fallback,
preference changes during review, and reviewed cancellation/reconnect/shutdown.
The full GPU flow separately verifies initially expanded review and untouched
one-second fallback when advanced display is disabled. Native Wayland Deny passed
at `.verification/native/20261015T102815Z-983c60a5/result.json` after 7.832 seconds;
X11 Allow passed at `.verification/native/20261015T102853Z-f7c2df12/result.json`
after 6.743 seconds. Both exceed the isolated five-second ordinary timeout,
require visible Reviewing/paused feedback, persist an actual input decision and
confirm full cleanup. Native pointer opening and other compositors remain
unverified; the background GPU journey proves pointer review. Early GPU failures
caught Tab resolving as a Kit action before element key listeners; the final
window-scoped keystroke interceptor keeps native actions intact. Failed evidence
is retained. The aggregate is `.verification/countdown-result.json`; checksum-change
warnings and the remaining parity inventory still need implementation/proof.


Checksum-review evidence: `.verification/checksums-core-final.log` (55 core
tests), `.verification/checksums-clippy-release.log`,
`.verification/checksums-build-final.log`,
`.verification/headless/20261015T104652Z-70bc9bd7/result.json` (nine focused frames)
and `.verification/headless/20261015T104258Z-5fa592de/result.json` (48 full frames).
The full journey preceded the final page-budget adjustment; the final core and
focused runs cover the 512 KiB bound including allocation capacities. Installed
1.7.2 `dialogs/prompt/_checksums.py` and `_display_checksums_warning` define the
warning / selected-or-all replacement workflow. Rustleblower compares valid
reported MD5/SHA1 with enabled confirmed flat AND rules for the explicit executable
and exact hash operands. Missing/malformed/unreported hashes, regex/list-file
predicates and unconfirmed popup proposals do not become replacement targets.

The GPU journey checks saved/reported values, paging, cancel/confirm, unchanged
cache while ACK is held, immediate independent Allow once, Stop before unsent
commands, an ERROR with persistent uncertainty and unchanged last-confirmed cache,
fresh inventory/reopening, and all remaining 33 rules across pages. All 35 saved
rules must preserve metadata/action/duration/unrelated conditions, including the
protocol JSON/list representation. The second permission still receives Deny
once, and the warning clears after successful updates. Core proof adds missing
fields, sensitivity, disabled/unrelated rules, count/byte limits, proposal exclusion
and stale/deleted/wrong-node preparation. One read/prepare permit, one review
window, 16-rule / 512 KiB pages, 512 KiB read cache and sequential acknowledged
commands keep state bounded. Permission handling never waits on these reads.

Native Wayland warning → separate review → return-to-prompt input passed at
`.verification/native/20261015T104324Z-4b1907d1/result.json`. The report requires
saved/reported OCR, zero mutation commands, a real reply, persisted decision and
full cleanup. It predates only the final page-budget adjustment. Native selected/
all update buttons, close/minimize behavior and other compositors still need
separate proof; opening the window is not full native mutation proof. No real
OpenSnitch daemon or firewall policy was changed. External daemon edits remain
subject to its last-writer protocol semantics; local rereads cannot provide an
absent daemon compare-and-swap. Remaining parity work is still listed above.

Native X11 opening/read/return proof on the final desktop binary also passed at
`.verification/native/20261015T104729Z-e86d3a0d/result.json`; its fixture likewise
records zero mutation commands, a persisted input decision and full cleanup.
The aggregate is `.verification/checksums-result.json`; the feature contract is
`.agent/skills/verify-rustleblower/features/checksums.md`.


History-tools evidence: `.verification/history-tools-core-release.log` (58 core
tests), `.verification/history-tools-export-final.log` (three targeted final
migration/filter/export checks), `.verification/history-tools-clippy-complete.log`,
`.verification/history-tools-build-complete.log`, and
`.verification/headless/20261015T111611Z-baa1749f/result.json` (48 full frames).
Installed 1.7.2 `dialogs/stats.py` defines fourteen general columns and CSV export;
Rustleblower now exposes those plus duration, logging, priority, reason and
snapshot-shortening status. Pinned Kit DataTable shares one 200-row Arc page with
the compact list, rather than duplicating all payloads. Eight typed AND conditions
use bound parameters; Reject has its own quick filter. Header sorts cover all
matches with a value/ID keyset and fixed arrival ceiling for older pages.

Schema 4 stores bounded searchable metadata and backfills 32 old snapshots at a
time on the history worker, independently of RPC decisions and listener startup.
NULL markers resume unfinished indexing; missing old values remain unknown rather
than UID zero. Indexing/error feedback remains explicit, and metadata sorting waits
for completion. Usage accounting includes the new data, with retention between
committed chunks. Legacy mid-backfill interruption is source-backed but is not yet
automatically exercised; completed schema-3 migration and legacy unknowns are tested.
A 4-KiB history settings file persists selected column order and compact/table mode;
widths survive refresh for the current view, but width/sort/filter persistence and
column drag reordering remain pending.

The focused GPU journey verifies 144 rejected reports, UID=1001 AND command
Contains blue1 yielding 108 rows, invalid port preservation, saved/reopened columns,
actual descending/ascending header input, sorted next-page navigation, CSV/JSON
files and clipboard contents, and existing-target preservation. Export uses selected
columns and captured search/action/sort with the form's conditions. A private
SQLite backup releases the live reader before output encoding; remaining old
metadata is backfilled privately. It streams rows, caps CSV/JSON output at 256 MiB
and clipboard at 4 MiB, checks Stop, and reserves active DB/WAL/SHM/preferences paths.
Snapshot acquisition has a 15-second limit, source cap 4 GiB and private copy cap
8 GiB; temporary disk use is additional to history retention. Cap-sized output,
native export stop/pickers, horizontal scrolling/resize gestures and native column
controls still need separate proof.

Native UID-filter input passed on Wayland at
`.verification/native/20261015T111842Z-6c41aeda/result.json` and X11 on the final
binary at `.verification/native/20261015T112813Z-17eac6f3/result.json`. Both require
UID/Equals selection, typing 9999, Add/Apply, main empty-state OCR, all 430 fixture
rows persisted and full cleanup. Early failures exposed driver key assumptions:
Enter opens the pinned Select, Space activates Buttons, and focus changes require
rendered frames. Failed runs are retained. Native proof does not imply file-picker
or export/column-button proof. No real daemon or firewall policy was changed.

The final focused ten-frame history-tools flow passed at
`.verification/headless/20261015T112931Z-8402ca57/result.json`, retaining the actual
108-row CSV/JSON Lines exports. The aggregate is
`.verification/history-tools-result.json`. Statistics and the other pending
inventory rows remain part of the active full-port goal.


Statistics evidence: `.verification/statistics-core-final.log` (64 core tests),
`.verification/statistics-core-release.log` (four final targeted protocol/storage
checks), `.verification/statistics-native-helper.log` (two input-helper guards),
`.verification/statistics-clippy-release.log` and
`.verification/statistics-build-complete.log`. The full 48-frame regression passed
at `.verification/headless/20261015T120224Z-806609e7/result.json`; the final focused
ten-frame statistics flow passed at
`.verification/headless/20261015T120634Z-d8031e96/result.json`. The full flow
predates the final history-refresh correction; the targeted core and focused
GPU runs cover that production revision. A subsequent small focus restoration
fix chooses the active search when a rule editor closes on Statistics; the final
build/Clippy include it, while these input journeys predate that close branch.

Pinned 1.7.2 `daemon/statistics/stats.go`, installed `service.py` and
`dialogs/stats.py` establish separate absolute maps, retained omitted keys,
category clearing and history drilldown. Schema 5 caches counters separately
from history, stores raw u64 as fixed-width decimal TEXT and aggregates exact
u128 values. Counter reports have a 10,000-key / 2-MiB allocation bound sharing
normal ingestion permits, with one latest pending sample. Cache rows are bounded
at 10,000 / approximately 2 MiB plus up to 32 small node records and expire on
subsequent reports. Reserving cache bytes from history's quarter-target logical
budget can evict older history; those deletions now publish a history revision.
Statistics skip/error/eviction health remains separate from incomplete-history
loss. Lower uptime starts that node's fresh counter lifetime, without clearing
events. Local resets persist baselines, preserve replayed zero and show future
increments; raw decreases clear the per-key baseline. No daemon reset or policy
command is issued.

The GPU journey uses real gRPC counter reports and actual Kit input for every
category, node/sort selectors, chart, stable second-page navigation during new
reports, Latest, existing-target refusal, all-match CSV/JSON, page clipboard,
exact history drilldown, cancel/selected/category reset and explicit matching
history removal. A held-writer core regression coalesces 20 reports, returns
Ping without awaiting SQLite, answers Allow once within 500 ms while the lock is
still held, and then persists the latest absolute sample. Module tests force
row/byte/node/age eviction and reject oversized retained capacities. Snapshot
capture uses one background permit / 512-KiB cache, a 4-MiB private file and
200-row pages. Current/pending/export may keep at most three files. Export is
bounded at 16 MiB / clipboard 4 MiB and shares history's reserved-path and atomic
publication guards; cap-sized output and long-duration RSS are not yet measured.

Native X11 `.verification/native/20261015T120708Z-17780efc/result.json` and
Wayland `.verification/native/20261015T120737Z-aaed39c4/result.json` require
Ctrl+3 opening, painted daemon totals/application values, keyboard empty-state
search, 430 retained events, three application counters summing to 430, zero
policy commands and full owned cleanup. They also predate only the final editor
close/focus adjustment. These do not prove native reset/chart/export/file-picker
gestures or real system-daemon enforcement. Cached-node browsing does not yet
provide simultaneous live node sessions. The aggregate is
`.verification/statistics-result.json`; the feature contract is
`.agent/skills/verify-rustleblower/features/statistics.md`. Nodes and daemon
preferences are the next inventory slices; all remaining parity and post-parity
upstream/Little Snitch research remain part of the active goal.


Node-session/control evidence: `.verification/nodes-core-complete.log` (69 core
tests), `.verification/nodes-native-helper.log` (two helper guards),
`.verification/nodes-clippy-complete.log`, and
`.verification/nodes-build-complete.log`. The full 48-frame GPU regression passed
at `.verification/headless/20261015T123414Z-da8595d5/result.json`. The focused
11-frame Nodes journey passed at
`.verification/headless/20261015T124028Z-e353ad87/result.json`; nine-frame checksum
regression passed at `.verification/headless/20261015T123723Z-1e142c18/result.json`
and ten-frame statistics regression at
`.verification/headless/20261015T123807Z-63452a12/result.json`. Full/checksum/
statistics precede the final legacy-cache cleanup; the 69-test core and focused
Nodes flow cover cleanup. The final compile/core also includes a subsequent
conservative node-payload reservation adjustment (512-byte fixed overhead).

Pinned 1.7.2 `daemon/ui/notifications.go`, `daemon/ui/config/config.go` and installed
`nodes.py` define the command contract. The daemon reloads configuration before
its final persistence reply, so ERROR leaves its state uncertain rather than
proving rollback. The new Kit Nodes page shows connected/cached summaries,
targets application rules, confirms Start/Stop interception and opens a separate
complete-JSON configuration editor with review/back/confirm. No control changes
the displayed confirmed state before matching OK and successful local cache.
The cache preserves unknown JSON fields, requires a 64-KiB object for changes and
stores at most 256 KiB of reported firewall protobuf per node. JSON has guarded
paste and post-change bounds in the pinned multiline Textarea. Typed preference
forms, changed-field presentation and native edit proof remain pending.

Session routing is now independent of the node selected for browsing. Up to 32
named Unix sessions retain their own notification channel, catalogue readiness,
uncertainty and permission ownership. Same-name subscription replaces only that
node; every Subscribe has a fresh generation, including reuse of one HTTP/2
transport. Old replies/cache operations and stream closure cannot affect a new
subscription or another node. Transport destruction releases even sessions that
never opened Notifications. All nodes share 16 pending commands, ten-second ACK
deadlines and the existing 8-MiB ingestion budget. Node controls serialize with
other commands for that node; refresh waits only for its prompts/commands.
Checksum review, history actions and already opened rule editors resolve their
own node/session. Permission popups explicitly identify their requesting node.

Schema 6 caches at most 32 metadata rows and loads only summaries until a node is
selected. A 512-KiB reader and one node read permit keep I/O off the UI thread.
Oldest offline metadata and its catalogue are evicted when needed. Metadata
admission also prunes unreachable legacy catalogues, while preserving current
live nodes whose metadata is still loading; a module regression tests both.
Rule catalogues retain their existing per-node count/byte limits and SQLite page
limits still apply. Statistics now has one fair latest FIFO slot per node, at
most 32 sharing normal permits, so one busy node cannot replace another node's
only pending report. Barrier/reset drains its bounded captured pending set.

Core proof routes same-named rules independently, rejects crossed ACKs, holds
alpha/beta commands and prompts, replaces alpha without disturbing beta, verifies
stale ACK/cache rejection and tests repeated Subscribe/old-stream close on one
transport. Interception/configuration tests check unchanged cache while ACK is
held, success, node-local ERROR uncertainty, unknown-field roundtrip, reopening,
invalid payloads and 32-node admission/offline eviction. The GPU flow selects
beta's actual catalogue, confirms alpha interception while answering beta's
actual AskRule, rejects nonobject JSON, edits/reviews/backs out/applies a complete
configuration, verifies payload and stored values, and preserves beta's last
confirmed state on ERROR. No user daemon or firewall is involved.

Native read/search/persistence passed on Wayland at
`.verification/native/20261015T123448Z-5e03ba91/result.json` and X11 at
`.verification/native/20261015T123731Z-589aacbb/result.json`, requiring metadata
name/kernel/interception/configuration, 430 events, zero policy commands and all
cleanup flags. Those runs precede only the final cache cleanup/payload-reservation
adjustments. Native mutation controls, node resource tasks, node removal/export,
address-based identity/TCP/TLS, cross-node bulk copying and typed preferences
remain open requirements. The aggregate is `.verification/nodes-result.json`;
the feature contract is `.agent/skills/verify-rustleblower/features/nodes.md`.
The full-port and subsequent upstream/Little Snitch research goal remains active.

Final desktop-binary Wayland Nodes read/search/persistence also passed at
`.verification/native/20261015T124415Z-20b3a7fa/result.json`, with zero commands and every cleanup
flag true. The aggregate records this final run.


Typed daemon preferences evidence: `.verification/daemon-preferences-core-final.log`
(74 passing core tests), `.verification/daemon-preferences-native-helper.log`,
`.verification/daemon-preferences-clippy-release.log`, and
`.verification/daemon-preferences-build-release.log`. The full 48-frame regression
passed at `.verification/headless/20261015T130821Z-169c58b9/result.json`. Adapted
Nodes JSON-mode regression passed at
`.verification/headless/20261015T130527Z-e942ce19/result.json` (11 frames). The
final focused typed-preferences journey passed at
`.verification/headless/20261015T131420Z-17a2e507/result.json` (11 frames).
Full/Nodes/core predate only the final UI failure polish that retains the reviewed
candidate after ERROR; the final focused journey, strict Clippy and desktop build
cover that revision.

The editor exposes 29 pinned Kit controls across General, Logging, Firewall &
memory, Statistics and Connection. Dirty-field patching preserves unknown fields,
precise numbers and absent defaults. Positive Go durations use exact nanosecond
range checks; queue/statistics integer and text/document bounds are validated.
Auth option values come from the installed UI's actual wire spellings rather than
inference from Go comments. Review retains up to 200 shortened before/after rows
and the complete bounded JSON candidate, and requires a separate confirmation.
No confirmed cache changes before matching OK and local persistence. Failure
keeps the reviewed candidate and last confirmed values; uncertainty blocks another
apply until refreshed. Permission input is proven while a configuration ACK waits.

The focused GPU journey edits every group, rejects a negative limit without a
command, exercises review/back/JSON/settings synchronization, checks the actual
CHANGE_CONFIG payload and independent SQLite reopen, answers Allow once while
ACK is held and preserves confirmed data on the next ERROR. The document remains
64 KiB, the form/control/subscription/scroll state bounded and permission handling
independent. Connection settings configure the daemon's client; Rustleblower's
server remains Unix-only. Apply-to-all-nodes, native interception and exhaustive
field variants, real TLS handshakes and real daemon enforcement remain pending.

Native default-action edit/review/save previously passed on Wayland at
`.verification/native/20261015T131337Z-e1c181b2/result.json`. The final binary passed at
`.verification/native/20261015T131729Z-86f02d2c/result.json`, including a painted
connected-node prerequisite, one acknowledged CHANGE_CONFIG=6, exact changed-only
JSON, 430 events and all cleanup flags. Native X11 preferences remain unproven. The later failed ownership-attach
run is retained with all cleanup flags true.

The daemon-preferences feature map documents exact contracts and commands at
`.agent/skills/verify-rustleblower/features/daemon-preferences.md`; aggregate proof
is `.verification/daemon-preferences-result.json`. Earlier failed native verifiers
are retained: small-path OCR misread DefaultAction, then the expected action number
was corrected from 2 to the protocol's CHANGE_CONFIG=6. Both were verifier failures;
the successful action and exact persisted JSON are independently checked. Tests
use isolated synthetic clients and never change the user's daemon or firewall.

The goal was paused after daemon preferences and subsequently resumed explicitly.
The inventory above retains the full remaining scope, including the later
OpenSnitch issues/PRs and Little Snitch research.

Alert archive evidence: `.verification/alerts-integrated-core.log` (79 tests),
`.verification/alerts-integrated-clippy.log`,
`.verification/alerts-integrated-build.log`, and
`.verification/headless/20261016T024001Z-bcfb22c4/result.json` (ten focused
frames). The full 48-frame application journey passed at
`.verification/headless/20261016T024148Z-54cbefca/result.json`.
Native Wayland opening/search/persistence passed at
`.verification/native/20261016T024230Z-f07e9a2c/result.json`: six exact synthetic
alerts, 430 history rows, zero policy commands and all owned cleanup flags.
An additional export check rejects active sockets/special files even when
Replace is chosen, and verifies stopped replacement preserves the original
file (`.verification/alerts-export-guard-sockets.log`). The first attempt lacked
sandbox socket permission and is retained separately.

The archive uses schema 7, a 2-MiB/10,000-row logical cap within the shared history
budget, bounded payload preparation and 200-summary pages. All five protocol
payload variants survive protobuf/JSON roundtrips. Node identity comes from the
sender's connection, independently of browsing selection. SAVE_TO_DB, SHOW_ALERT
and NONE are distinct; ACK confirms receipt, not durability. GPU input covers
read/deletion review, all-page CSV/JSON export, replacement/cancel and independent
permission replies. Matching deletion retains arrivals after its review ceiling.
Native proof covers opening/search only, not every mutation/export control.
The desktop notification slice below supplies a bounded transport; host-desktop
banner visibility and rule alert configuration remain pending. No real daemon,
existing socket or firewall policy was changed for this slice.

Desktop notification evidence: `.verification/notifications-core.log` (80 core
tests), `.verification/notifications-dbus-final.log` (private owned D-Bus worker
integration), `.verification/notifications-clippy-final.log`,
`.verification/notifications-build.log` and
`.verification/headless/20261016T025314Z-9bffb8b9/result.json` (four input frames).
The ten-frame archive regression also passes at
`.verification/headless/20261016T025237Z-bcdcbda7/result.json`. The aggregate
`.verification/notifications-result.json` records final source hashes and proof
boundaries, including the separate full application regression at
`.verification/headless/20261016T025317Z-50921ac5/result.json` (48 frames).

The worker has eight queued bounded summaries, one latest pending notice, one
known active ID and one pending Open action. It coalesces bursts and replaces
known IDs at most twice per second. Exact urgency, escaped text, lifetime, Open
action, spoof/stale rejection, user dismissal, closing on mute, timeout recovery
and cancellation are tested against a private notification service. A held Notify
reply cannot gate real Ping or Allow replies, each required within 500 ms.
GPU input saves/reopens the toggle, preserves archive recording while muted and
restores the Alerts dashboard with a separate permission still pending, then
answers through actual input. Legacy preference versions 1–3 migrate enabled;
version 4 retains explicit choices and rejects invalid booleans.

Pinned GPUI Linux notifications were inspected first: their per-notice waiting
thread and missing urgency/retraction do not meet the chosen resource bounds.
The resolved zbus version is reused by a single worker, keeping the existing GPUI
backend and permission path. Service acceptance does not prove desktop visibility;
Do Not Disturb and server expiry remain system choices. Host-compositor banners,
physical notification clicks, permission-notification alternatives, other
lifecycle notices and rule alert configuration remain pending parity work.
Prior tray, prompt/icon, placement and choice implementation files remain
byte-identical to the saved integration baseline; their shared composition and
preferences receive only this feature's additions. The isolated alert worktree
was archived after preserving its ignored evidence under
`.verification/alerts-isolated/`. No changes from this slice were committed or
published.

Live inspection evidence: `.verification/live-core-final.log` (87 tests),
`.verification/live-clippy-final.log`, `.verification/live-build-final.log`,
`.verification/live-helpers-final.log`, and
`.verification/headless/20261016T031322Z-4758df4d/result.json` (eleven frames).
The full 48-frame application regression passes at
`.verification/headless/20261016T031326Z-fb228fb8/result.json`. Native Wayland
passes at `.verification/native/20261016T031559Z-0debe830/result.json`: actual
Ctrl+6/Start/search/Pause, three painted synthetic socket rows, an empty search,
Stop requested feedback, exact TASK_START/TASK_STOP configuration, 430 history
rows and every owned cleanup flag. Aggregate source hashes and boundaries are
in `.verification/live-result.json`.

The daemon's task handlers stream replies under the original start ID and do
not acknowledge successful stop. Four bounded tasks use the shared ID allocator
without consuming any of the sixteen policy-confirmation slots. Each keeps one
latest 1-MiB snapshot; parsing uses one background permit, bounded raw visitors,
a 4-MiB socket view and 200-row pages. Errors/oversize/stale/disconnect stop or
expire only the inspection, preserve readable last data and leave policy cache
certainty intact. Full channels keep an owned stop retry. Data retains its
source node; new starts use the current selected generation. No task reads the
UI host's /proc as a substitute for daemon-provided results.

GPU input proves protocol/family/state/interval filters, 225-socket paging and
113 matching filtered sockets, selected PID details, readable environment and
descriptors, maps, complete/unknown-number clipboard data, error preservation,
Sysinfo units/load, a real permission while the resource update waits, and five
starts/five stop requests including close/restoration. First attempts exposed
missing searchable Select inputs, mode-change focus loss and an asynchronous
stop-observation race; passing input paths and explicit observation cover the
corrections. Failed evidence is preserved. A later capacity-test fixture needed
a valid match operator; the final core suite verifies sixteen valid commands
remain available alongside four live tasks.

The pinned daemon initializes tasks asynchronously and stop completion has no
wire confirmation; source inspection does not prove every daemon-side restart
or kernel-lifecycle race. Native process fields/pickers/paging/copy, actual
kernel/process visibility, direct titlebar minimization, live export and saved
filters/columns, history-to-live shortcuts and long-duration RSS remain explicit
parity/proof gaps. No changes were committed or published for this slice.

The older alert-integration hash baseline differs from current tray/prompt-window
files; those files were retained and not edited by the live slice. Do not treat
that older baseline comparison as a claim of byte identity for this turn. The
native report identifies its owned binary snapshot independently.

History-preferences evidence in the isolated working copy: full core regression
92 tests plus a separately added passing schema-7 migration test, strict Clippy,
ten-frame focused GPU (`20261016T033123Z-e71ef5b7`), ten-frame history-tools
regression (`20261016T033242Z-0c450a5a`) and full 48-frame application regression
(`20261016T033239Z-3c3af45c`). The real idle-cleanup test waited one minute with
no new ingestion; transaction failure restored both data and settings. A held
SQLite write lock still allowed real Ping and Allow replies within 500 ms.

Native Wayland retention review/save passed at
`.verification/native/hp-20261016-0340-c912/result.json`: days=1, unchanged
256-MiB target and five-minute cleanup, 430 fixture rows, no policy commands and
all cleanup flags. Earlier native attempts are preserved: the nested input socket
path exceeded the Unix limit, then focus ownership changed and the guard refused
further input. Retrying with short evidence path, explicit owned-window attachment
and per-step logging passed. Native display controls and other fields are separate
proof gaps. The initial GPU attempt used only virtual time for a real interval;
the corrected journey waits two real seconds and passes automatic refresh.

Schema 8 stores scalar retention/revision in SQLite, saves pruning atomically,
rejects stale reviews and reports saved-with-maintenance-warning after commit.
View document version 2 migrates one-second defaults and merges refresh/column
writes independently. Explicit CLI flags override saved retention; implicit defaults
do not. No in-memory history mode or implicit database migration was introduced.
Requested targets remain distinct from physical allocation and pending compaction.
Database relocation/backend/journal controls, temporary-rule filtering and saved
widths/sorts/filter presets remain unfinished. Root integration evidence and final
source hashes are recorded in `.verification/history-preferences-result.json`.

Integrated history-preferences checks completed: 93 core tests, ten-frame focused
GPU journey, strict Clippy, formatting, two Python helper checks and desktop build.
The aggregate report retains source hashes and separates root integration checks
from the isolated full/history-tools regressions and native Wayland evidence.

History-to-process inspection is implemented from connection details. Requests
capture the event node's current generation and positive PID in a four-entry
action queue, then revalidate it before creating a daemon-owned read task. Live
selects that event node explicitly, clears unrelated search state and keeps the
recorded event/time/path with a PID-reuse/current-data warning. Unknown/missing
PIDs and disconnected nodes cannot start tasks; a queued replacement is rejected.
The focused eight-frame GPU fixture uses two nodes, holds a task reply while an
independent real permission is answered, copies a different current executable,
and verifies process-error retention, invalid/offline guards and stale-session
rejection. Native shortcut input remains a separate pending proof.

A first Live regression used two initial updates without subsequent reporting,
so the long field-navigation flow could legitimately hit the production silence
watchdog. Its failed evidence is preserved. The fixture now streams at the
requested interval, owns/cancels its periodic tasks, and pauses before selecting
a socket for stable copy/inspection. The production watchdog is unchanged.

History-to-process final evidence: `.verification/history-process-result.json`
records eight focused GPU frames, full 48-frame and Live eleven-frame regressions,
strict all-target/all-feature Clippy, formatting, verifier syntax and desktop
build. The earlier 93-test core result remains applicable to unchanged core
modules; this UI slice did not rerun it. A source-backed full system-firewall
contract/sequence is in `docs/system-firewall-plan.md`; implementation remains
pending and the full parity goal stays active. No new changes were committed or
published by these slices.

System-firewall foundation evidence: `.verification/firewall-result.json` records
a 97-test complete core suite, subsequently extended focused compatibility tests,
the final eight-frame nftables GPU journey, eleven-frame Nodes regression and
48-frame general regression, strict Clippy, formatting and desktop rebuild.
The accepted cache has schema-9 provenance; a failed marker write rolls back both
state changes, and fresh reports replace accepted provenance. Protocol-only
fixtures cover crossed ACKs, stale review/session/backend/configuration guards,
maximum uint64, exact complete protobuf and independent Ping/Allow.

The GPU journey validates 225 rules, paging/search, invalid JSON, oversized real
paste preservation, actual before/after paths, Back, held ACK with independent
permission, reader contention across ACK, clipboard and rejected draft/cache
preservation. Initial attempts are retained: the Textarea accessibility label
was not a test ElementId, long input rendered per character, and an immediate
post-ACK read could fail admission and retain an old snapshot. Explicit focus,
real clipboard paste and a globally bounded asynchronous read wait corrected
those paths. Interrupted owned fixtures were cleaned; evidence metadata states
which database directory remained available to preserve. Final backend gating
requires nftables format 1 and invalidates the previous report after daemon
configuration changes. Native firewall controls, dedicated forms/profiles,
enable-disable sequencing and actual enforcement remain unfinished.

System-firewall field-editor evidence is in `.verification/firewall-forms-result.json`.
The complete core suite passes 98 tests, including atomic chain/rule commands,
legacy-field preservation, ID collision rejection, no-op order preservation and
field-level added-object review. The thirteen-frame GPU journey exercises native
Kit fields, nested expression/value keeps, local staging and recovery, exact full
protobuf and independent permission input. The fixture carries a near-cap wire
value and rejects an additional 8-KiB value before growing the staged rule beyond
its budget. This is an interactive bound check, not a long-duration RSS claim.

Owned Wayland proof `.verification/native/ff-20261016-0456-a73c/result.json` passes
chain-name editing through Alt+E, keep/review through Ctrl+Enter and acknowledged
apply through Ctrl+Shift+Enter. Independent Python protobuf/cache checks retain
every other field, one rule, a 32-byte accepted marker and 430 events. All five
cleanup flags pass. Rule/expression/value native controls remain separate proof
work. No host daemon, socket or firewall was changed.

The headless wrapper now selects exact test names: adding headless_firewall_forms
made the old headless_firewall prefix select two tests sharing one evidence path.
The incomplete wrapper run is preserved with observation metadata; the exact
eight-frame JSON regression passes. The scoped file map and unimplemented profile,
file, enable-disable and backend-condition workflows remain explicit.

A final pinned-source check corrected the initial row-position assumption.
Installed `firewall/rules.py::swap` changes array order without rewriting Position.
`daemon/firewall/nftables/system.go::AddSystemRules` iterates rules in reverse;
`nftables/rules.go::InsertRule` forwards Position to the nftables rule. The final
editor therefore preserves insertion metadata on moves/duplicates and defaults
new rules to zero. Earlier GPU runs expecting normalized row positions are kept
as historical evidence and superseded by the corrected focused run. Form commands
also reject enabling an expression-free rule, which AddSystemRule would ignore.
Exact uint64 values remain available as explicit advanced metadata; synthetic
wire-boundary tests do not claim those values are valid kernel insertion positions.

The field-form checkpoint was finalized with 98 core tests and native Wayland
`.verification/native/ff-20261016-0512-b94d/result.json`; the restored desktop binary
matches that tested executable's SHA-256. Historical earlier native reports remain
preserved. `.verification/firewall-forms-result.json` records the exact boundaries.

The next file/profile slice adds typed Kit paths and native prompt entry points,
create-new full/chain export, profile clipboard, full local-draft replacement,
profile matching/deduplication/removal and bounded on-disk profile discovery. The
installed 1.7.2 `firewall/__init__.py::apply_profile` and `rules.py::is_duplicated`
define chain identity and UUID-agnostic complete-rule duplicate comparison.
An imported profile retains chain name/priority, applies its policy and appends
new rules. Removing its exact rule IDs retains policy instead of inventing an
accept transition. Complete before/after review remains mandatory before any
RELOAD_FW_RULES command. Built-in policy presets and enable/disable are unfinished.

Core tests cover exact JSON/profile roundtrip, maximum uint64, unknown fields,
regular-file/size/container/catalogue bounds, no-overwrite exports, duplicate IDs
and content, chain matching and atomic rollback. The `firewall-files` GPU journey
requires eleven frames and adds two profile-list pages, actual path inputs and
buttons, rejected import preservation, import/export/copy, Review/Back/keyboard,
exact complete protobuf, independent permission under held ACK, accepted cache
and offline export. Native OS picker interaction remains unverified. A first GPU
run exposed focus restoration to the hidden textarea on Back; the corrected path
focuses the visible page and its keyboard review succeeds. Preserve both reports.

A final compatibility check found the predefined-file envelope in installed
`dialogs/firewall.py`: it displays outer `Name` and passes `Profile` to the wire
parser. Transfer now accepts both that envelope and bare FwChain JSON, and exports
an envelope suitable for OpenSnitch's predefined-profile loader. Unknown fields
inside the policy payload still fail; outer display metadata is ignored as in
the original UI. The corrected GPU run and independent installed Python protobuf
parse supersede the earlier bare-only interchange checkpoint.

Current file/profile evidence: `.verification/firewall-files-result.json` links the
final eleven-frame envelope-compatible GPU flow, the 13-frame form and 8-frame
JSON regressions, strict Clippy, formatting, a completed 100-test full core run,
then six transfer/protocol tests after the isolated envelope correction, and a
restored desktop build. Installed Python protobuf independently parses both
exports and requires exact chain equality, legacy wrapper retention and uint64
maximum values. No user's daemon, socket or kernel firewall was used. The OS
picker buttons use the existing asynchronous API but native dialog interaction
is not yet proven. Full firewall and broader port parity remain incomplete.

The upstream baseline was explicitly advanced to retained bare-repository commit
`a1353848ba1b660320e90cefea782c3fba272c00`. New master behavior and priorities are
tracked in `docs/upstream-reference.md`: GUI-side list subscriptions, port ranges,
domain wildcard/glob and mixed IP/CIDR lists, table/search/export workflows,
reviewed multi-node rule operations and opt-in persisted interception/pause state.
The daemon downloader is unchanged since 1.8.0 and is not the subscription manager.
Schema identity does not imply semantic identity. None of these new gaps is marked
complete by the existing JSON/rule/task coverage.

User-requested final phase: after functional parity/additions, complete the
cohesive native UI pass in `docs/ui-consistency-plan.md`. Non-sortable tables,
weak semantic color and inconsistent page layouts are explicit current concerns.
Sort meaningful columns across the full matching dataset, preserve bounds and
context, and verify shared page/edit/action/status patterns across actual screens.
Subscriptions must be directly discoverable and create configuration through a
usable empty screen and normal UI actions; no manual JSON bootstrap is required.
This phase remains required before marking the full goal achieved.

System-firewall policy/state checkpoint: selected-chain Accept/Drop presets retain
actual chain metadata and unrelated rules; ordinary edits and Enabled changes
share an exact bounded step review. Disabling first prepares accept policies on
the captured original base chains (without activating newly drafted rules), waits
for ACK and durable cache save, then writes the complete disabled configuration.
One global operation lease prevents same-node interleaving and stays independent
of the editor window and permission path. Rejection, cache failure, reconnect or
real timeout stops continuation, preserving earlier accepted stages and drafts.

The current core suite passes 102 tests, including eight firewall tests with
first/second-stage rejection, cache failure, reconnect and a real ten-second
command timeout. Background regressions for file/profile, field forms and JSON
pass. Owned native Wayland report
`.verification/native/policy-20261016-0605-7a1c/result.json` proves Alt+P,
Ctrl+Shift+E, the complete two-stage review, exactly two type-5 commands, preserved
rule fields, final Disabled/accept state, accepted-cache marker and 430 events;
all five cleanup flags pass. The first native attempt is preserved: OCR missed
the digit 1 in Step 1/2 even though the rendered review was correct. The verifier
now requires both distinct step descriptions plus Step 2/2 and independently
compares both full protobuf payloads; it does not infer success from OCR alone.
Native policy proof is not kernel enforcement or complete backend-helper parity.

The strengthened eleven-frame policy journey closes the editor before replying
to the preparation command, then requires the exact second command and final
cache state. Frame 08 captures the remaining dashboard, not a closed editor.
`.verification/firewall-policy-result.json` aggregates final results and preserved
failed verifier attempts. The rebuilt current binary matches the successful native
instance's SHA-256. Final UI/UX consistency remains a required later phase.


### Native blocklist catalogue/download workflow — partial parity

The current upstream subscription priority now has an implemented first native
workflow, reached through Blocklists/Ctrl+7 with a usable empty page and normal
Add/Edit/default controls. UI actions create the config; no manual JSON bootstrap
is required. The separate bounded SQLite/HTTP worker owns metadata, schedule,
status, validators, downloaded files, cancellation and last-good data without
blocking telemetry or prompts. Explicit start/pause is persisted; automatic work
waits for a local session. Removal retains data and allows restoration.

Local verification now includes 109 core tests (seven subscription cases), strict
Clippy, eight Python helper guards, 14 subscription GPU frames, the 48-frame
existing application flow and three native Wayland input/save frames. See
[subscription proof/bounds](../.agent/skills/verify-rustleblower/features/subscriptions.md)
and `.verification/subscriptions-result.json` for exact provenance and limits.
HTTP fixtures run in an owned network namespace; native proof creates metadata
with downloads paused. No system daemon, firewall policy or real UI socket was
changed for tests. The GUI input failures found during development (Select wrapper
height, focus restoration and French Canadian punctuation) have retained evidence
and verified corrections.

This is not full subscription parity. Next implement canonical daemon list files
and reviewed attached-rule creation/inspection for explicit local-node targets,
then bulk/groups, config/action interchange, notifications and retained-file
cleanup/crash recovery. Source-derived wildcard/glob/mixed-IP matching proof is
required before claiming that those lists block traffic. Port ranges remain the
next upstream-delta priority after subscriptions; the final whole-app UI/UX pass
and subsequent issue/PR/Little Snitch research remain in the complete goal.


### Single-subscription canonical data and associated rules — partial parity

The next slice publishes stream-normalized daemon files in stable typed owned
directories, with a publication registry and disk accounting for retained old
kinds. It adds reviewed local-node watched-directory drafts, exact operand/path
mapping, confirmed-only associated-rule inspection over all inventory pages and
editing through the normal acknowledged editor. The target node is explicit in
that editor. Rule review/cancel/ACK/error and prompt isolation are exercised with
synthetic gRPC commands, alongside source-backed Go reader/matcher/monitor proof.
The full local core suite is now 115 tests; subscription GPU coverage is twenty
frames, with separate general/shared-editor regression and native metadata setup.
Exact results and source identities are in `.verification/subscription-rules-result.json`.

Next subscription work remains group/all-list directories and activation semantics,
bulk configuration, action import/export, notification preferences, old-format
inspection/cleanup and filesystem/database crash recovery. This slice does not
prove kernel enforcement, hosted CI, all regex/glob syntax, external HTTPS,
packaged input or lower RSS. Port ranges, all remaining UI parity, final cohesive
UI/UX and the requested issue/PR/Little Snitch research remain in the active goal.


### Collection service foundation and corrected local-user rule scope

Source inspection found that subscription rules default to the local user's UID.
The shared single-list/collection factory now includes that exact UID predicate,
with independent kernel-UID and actual GPU/wire proof. The typed group/all service
now builds bounded active memberships in owned generations, uses stable reload
markers plus a persisted publication epoch, retains canonical source data, and
refuses unknown pointer/directory state. Core queries cover all collection pages.
The product's group picker, reviewed membership mutations and associated group
rule controls remain required next. See
[the collection plan](subscription-collections-plan.md) and
`.verification/subscription-collections-result.json`; the full goal stays active.


### Native collection picker, associated targets and membership review

Blocklists now exposes Rule collections/Ctrl+Shift+L with usable empty all-list
kinds, exact group/kind resolution, member counts, full-catalogue search and every
Group/Type/Active sources sort direction. Rule review captures the selected local
node and UID-AND-directory predicate. Associated inspections retain an explicit
target during paging, reread entries before editing, refuse changed snapshots
and refresh on request. Saved source membership edits and restoration now require
before/after review with preserved Back/Cancel drafts; removal text distinguishes
active collections from retained direct-list data.

The local proof includes 122 core tests, 32 collection/subscription GPU frames,
the 48-frame general regression, strict Clippy/eight helper tests, and four owned
Wayland review/navigation captures with UID 1000, exact long-directory inspection,
Cancel/filter focus restoration and complete cleanup. The titlebar close action
minimizes the generic editor; the native verifier uses its actual Cancel button.
See `.verification/collection-ui-result.json` and the subscription feature map.
Native saving/membership/paging, interrupted-query lifecycle, bulk assignment,
action/config interchange, notifications and other full-parity gaps remain open.
The complete goal, final UI consistency phase and later feature research remain
active; this is not completion of the whole subscription port or product.


### Reviewed bulk subscription configuration

The pinned upstream bulk controller edits enabled state, groups, input format,
interval, timeout and maximum size. The native equivalent now exposes all six,
plus additive/removal/clear group operations and explicit per-source inheritance.
Marks survive sorting/paging; search/removed-mode changes reset them. A captured
filter can cover every active matching source. Review freezes the exact changed
entries, shows before/after values in 16-entry pages, preserves fields on Back
and writes nothing on Cancel. Rebuild deliberately resolves fresh scope/state.

One bounded SQLite transaction checks every changed revision/configuration before
writing. A stale/removed target or any SQL failure rejects/rolls back the complete
metadata batch. Only changed revisions advance, validators clear only for input-
format changes, and successful changes synchronize collections once. Existing
policy definitions and retained direct-list files are preserved. Filesystem/DB
crash atomicity remains a separate recovery gap.

Local evidence: 125 core tests including three new bulk cases, the complete
39-frame subscription GPU/input journey, strict Clippy/eight Python guards and
eight owned native Wayland captures. The core test holds a real subscription SQL
write transaction while production Ping and AskRule/Allow complete within their
500-ms bounds. Faults on the last UPDATE roll back earlier rows and logs. Native
input proves a reviewed true→false mutation and exact persisted unchanged fields,
paused/no-download state, zero policy commands, 430 history rows and full cleanup.
Evidence and current source hashes are in `.verification/subscription-bulk-result.json`.
The subscription map distinguishes core, GPU and native scope. Failed native/GPU
attempts are retained; selector text/timing and review focus were corrected.

Action/config import/export/migration, subscription notifications, retained-data
inspection/cleanup, interrupted-query lifecycle and broader transport/resource
proof remain pending. Port ranges and the entire original parity scope, final
cohesive UI/UX phase, upstream issue/PR and Little Snitch research remain active.


### Bounded configuration import/export and source-backed migration

Blocklists exposes Import / export and Ctrl+Shift+T (Ctrl+Shift+I is reserved by
the pinned Kit inspector). It accepts native version-1 configuration, an
OpenSnitch subscription action envelope or its config object. Import normalizes
units, compact durations, filename derivation/type suffixes/collisions and exact
legacy-group grammar. Review has explicit name replacement/restoration and
optional defaults/automatic-update state, paged before/after values, preserved
Back/Cancel options, stale refusal and explicit Rebuild. The exact captured
review is applied even if its source file changes afterwards.

One transaction revalidates catalogue, settings and template revisions before
writing; late SQL failures roll back new rows, updates, logs, defaults and opaque
metadata. Source files/directories and cache publications are not adopted.
Unknown metadata is preserved for export and never executed; per-source mapping
currently follows names and needs stable ID rebinding for rename fidelity.
Exports use consistent read snapshots, selected or full-filter active scope,
private temporary files, fsync and no-clobber publication. Embedded templates are
filtered to selected records. Native global limits use seconds/bytes; legacy
unit-only overrides become explicit native values at migration time.

The retained exact upstream Python models/helpers accept the real worker export
and match nine actual Rust normalization fixtures. The four new core cases cover
preview/no write, conflict/restoration, stale catalogue, late SQL rollback,
changed-file/captured review, restart, metadata, 70-row filtered exports,
source/destination guards, malformed/oversized JSON and strict capacity. The full
core suite passes 129 tests. The subscription input journey now has 48 captures,
including invalid/config conflict review, defaults, review paging, Back/Cancel,
stale refusal/rebuild, retained bytes and selected native/full-filter action
exports. General 48-frame GPU, strict Clippy/eight Python guards and three owned
Wayland import captures are separate evidence layers. Native persistence requires
native-test/120-second/2-KiB conversion, unchanged defaults, no downloads/policy
commands, 430 history rows and complete cleanup. See
`.verification/subscription-interchange-result.json` and the current feature map.

The full port remains active. Remaining migration work includes authoritative
catalogue replacement, duplicate/ambiguous identities, stable per-source metadata
across renames and legacy unit-only coupling after later default changes.
Subscription notifications, retained-format inspection/cleanup, crash recovery,
transport/resource/package proof, port ranges, every remaining parity feature,
final coherent whole-app UI/UX and requested issue/PR/Little Snitch research are
still required. These local fixture checks do not prove real daemon enforcement.


## Per-ID subscription metadata and complete catalogue replacement

Native version-1 and OpenSnitch transfers now allow duplicate display names.
Unknown source fields are stored by immutable subscription ID and survive rename,
removal, restore and restart. Native exports retain exact names/filenames;
OpenSnitch exports retain labels with collision-free reader-typed filenames.
Ordered native embedded identities are validated before review. A one-time
transaction migrates older name-indexed metadata; uncertain/orphan records stay
in a bounded archive with a visible count. Reimport reconciles exact archived
source records without guessing an attachment.

Complete active-catalogue replacement is an explicit choice. Review includes
both imported rows and every absent active identity; removals retain downloaded
files and existing daemon definitions. Cancel leaves the catalogue intact.
Matching, settings and extra fields are revalidated in one transaction; late
removal errors roll back prior inserts, metadata, defaults and logs. Successful
commits cancel affected downloads and synchronize collections once. The archive
count is read before commit and published without SQLite under the UI-state lock.
Aggregate reviews are bounded at 6 MiB before queueing. Global/bound/archive
metadata together remain capped at 2 MiB with at most 256 bound/archive rows each.

Evidence: 133 core tests (eight transfer/identity cases), 56 subscription GPU
frames, 48 general GPU frames, three owned Wayland replacement captures and
12 pure pinned Python codec checks. Native evidence independently requires two
same-name active identities, one retained removal, exact per-ID extra fields,
unchanged defaults, no checks/downloads/policy commands, 430 history rows and
complete owned cleanup. See `.verification/subscription-identity-result.json`
and the current subscription feature map. These checks do not establish real
daemon enforcement, package/resource performance or filesystem/SQLite crash
atomicity.

The full port remains active: archive inspection/manual resolution/cleanup,
legacy unit-only coupling, notifications, retained-format controls, recovery,
transport/resource/package proof and other subscription gaps remain. Port
ranges, multi-node workflows, persisted interception, all other parity items,
final whole-app UI/UX and upstream issue/PR/Little Snitch research still apply.


## Inclusive source/destination port ranges

The native editor now supports the pinned daemon's inclusive `range` operator
for both port fields, including AND rules, exact JSON/structured roundtrip and
unchanged original wire spelling. Validation follows Go's ASCII-space removal,
unsigned 64-bit decimal bounds and ordered endpoints; imports retain wider
numeric bounds even though actual ports stop at 65535. The shared command queue
rejects Change/Enable on old or unknown sessions. Support is established by a
reported 1.9+ version or an existing reported range rule, resets on replacement,
and is independent per session. A bare 1.8 string is ambiguous: source commit
e1b961ae added the type while master still advertised 1.8.0. Explicit compatibility
selection for otherwise unconfirmed backports and native compositor editing
remain follow-up work. Disable/Delete remain usable.

Three core cases cover exact grammar/import/export, early version reports,
current-session compatibility, ACK/error persistence and independent permission
replies. An isolated Go harness compares 21 actual Rust fixtures with unmodified
pinned compileRange/rangeCmp bodies and inclusive boundaries. The dedicated
8-frame GPU journey exercises actual Kit controls, scroll, JSON, command refusal,
held ACK, permission input, rejected drafts and clipboard roundtrip. Source/runtime
proof does not claim kernel enforcement or backport detection. See the current
application-rules verification feature and `.verification/port-range-result.json`.
The full port goal and all remaining parity/UI/research requirements stay active.


## Opt-in persisted interception pause

The selected Nodes page now exposes per-node pause memory, off by default.
It captures a confirmed reported state; later Start/Stop changes update the
remembered choice only after ACK and in the same SQLite transaction as node
metadata. It is UI-owned and does not rewrite daemon settings. Disabling it
leaves current interception alone. This scopes the pinned UI's single-node
global preference to Rustleblower's existing explicit node names.

A fresh connection reporting running can receive one automatic Disable after
its exact generation's metadata/inventory and command stream are ready. The
existing backend maintenance task performs this without a main window or disk
reads on RPC/UI threads. It never auto-starts a stopped daemon. Manual commands
supersede unsent restoration; normal uncertainty/busy/firewall limits apply.
Restoration stays busy through durable cache completion, not only daemon ACK.
Errors surface in status and wait for a fresh reconnect before another attempt.
There are at most 32 saved rows/4 KiB of names and 32 completion receivers; cache
eviction removes the matching preference. Invalid oversized stored snapshots
suspend restoration with visible feedback rather than using partial state.

Three transport tests and one unit test cover complete backend restart, per-node
and generation routing, crossed ACK/error, stale preference, late SQL rollback,
cache eviction/corruption and permission replies under both held ACK and a real
SQLite write lock after ACK. Full core has 140 cases. Nodes GPU now has 17 frames
covering actual opt-in, reconnect, held restore, permission, opt-out and manual
resume. Native checkbox persistence and the shifted daemon-preference keyboard
path have separate owned Wayland verification. See
`.verification/interception-result.json` and the Nodes feature map.

The full goal remains active. Remaining gaps include native interception
variants, remote peer identity/transport, cross-node copying, subscription runtime
and archival gaps, broad resource/package proof, final coherent whole-app UI/UX
and requested source-backed issue/PR/Little Snitch research. Local fixtures do
not prove policy enforcement by the real daemon.


## Reviewed copies of existing rules across nodes

Application rules now offers Copy to nodes with marked/highlighted or captured
full-filter scope, explicit destinations and opt-in name replacement. A short
consistent SQLite snapshot captures one bounded source vector and compact target
summaries/fingerprints. Identical and awaiting-observation names always skip.
The source is unchanged; path/UID/operator/wire metadata is preserved exactly.
Per-node results, copied full JSON and copied result text make the action
reviewable. There is no distributed rollback.

The engine reserves the current destination's app command path, fences prior
confirmed cache writes, and closes SQLite reads before waiting for an RPC.
Exact per-name/configuration/generation checks precede each mutation. A bounded
current-target hash map incorporates confirmed writes; Stop/retry keeps a cursor
and fingerprint without replaying successes or retaining every destination map.
Other nodes continue after an error. Unknown/reconnected targets require rebuilt
review; new review skips exact copies already confirmed. View destruction stops
unsent work after the current reply. Permissions and telemetry bypass the lease.

Three core cases cover source/selected scopes across 230 rules, conflicts,
identical/proposed exclusions, held ACK, independent permission, actual checkpoint
without a retained read snapshot, reserved command/refresh refusal, partial node
errors, stale edits/generations, range/capacity and oversized source guards.
Full core has 143 cases; focused final cases cover source-admission hardening.
A 14-frame actual GPU journey adds real marks over two pages, exact JSON, scopes,
Stop/resume, per-node error/results, reconnect/rebuild, explicit replacement and
forced owned-window destruction during ACK. Existing 48-frame GPU regression
also passes. See `.verification/cross-node-result.json` and the feature map.

This is existing-rule copy parity. Direct multi-node unsaved drafts/edit/rename,
multi-node file import/export, per-target operand diffs, native compositor copy
input and broad resource/package proof remain. The subscription runtime gaps,
all other parity work, final UI/UX pass and upstream issue/PR/Little Snitch
research remain in the active full objective.


## Multi-node unsaved drafts, ordered rename and imports

The shared rule editor now opens Apply to nodes from its current staged fields.
Selected destinations include the original node; same-name edits default to
explicit replacement. Cancel returns to the unchanged staged parent editor.
A rename reviews the original-removal count and confirms the complete new rule
before deleting the old name on each destination where it exists. The cursor
tracks removal independently: Stop after the new ACK retains both names and
continue sends only the deletion. A failed removal reports that the replacement
is already confirmed; reconnect/review recognizes it and avoids resending it.
New-name conflicts and awaiting originals cannot silently delete old rules.
Temporary catalogue capacity includes both names before removal.

Import to nodes reuses bounded rule file/directory/clipboard parsing, then freezes
one shared source vector for the reviewed destinations. Later file/clipboard
changes do not alter Apply. Invalid captured clipboard data cannot fall back to
a file path. No source copy is multiplied by the number of targets, no SQLite
read spans an ACK wait, and permissions retain their independent path.

All 146 core tests pass, including six cross-node cases. Three new cases cover
shared unsaved source allocation, ordered rename/Stop/resume, a late SQLite
delete failure with retained names and fresh-generation deletion-only recovery,
explicit collision replacement, imported inline values and invalid inputs.
The new 14-frame GPU journey covers actual editor/Cancel/destination controls,
rename/ACK/permission/Stop/continue, invalid imports and immutable file/clipboard
review. Existing 14-frame copy and 48-frame general regressions pass. Strict
Clippy and a desktop rebuild complete this slice's build evidence.
See `.verification/multi-node-drafts-result.json` for exact reports and hashes.

These are isolated synthetic protocol/cache/GPU proofs. Native OS pickers, new
compositor controls, real-daemon policy enforcement, package/RSS and complete
whole-app UX are not proved by them. Multi-node export, per-target operand
differences, remaining subscription/runtime/transport and other parity items,
the final UI/UX pass and subsequent issue/PR/Little Snitch research remain in
the active full goal. Historical sections above retain their original scope.


## Multi-node disk-backed folder export

Pinned OpenSnitch events/views.py and rules.py export one rule directory per
node. Export nodes now selects cached (including offline) identities and all
rules or the captured text/duration filter. One consistent source snapshot
copies bounded wire records into private sequential spools, releasing the live
SQLite reader before JSON encoding. The review holds one owned directory and
compact node summaries; later source changes do not affect the output.

Publish stages one importable JSON subfolder per node plus an identity manifest,
then atomically renames with NOREPLACE. Safe digest components distinguish
colliding node/rule names without changing document identities. Cancel and
pre-publication errors clean the staging tree; completed exports are retained.
Memory stays at one decoded rule/JSON document plus bounded SQLite buffers and
32 summaries. Wire spools are at most 64 MiB plus framing, JSON 256 MiB across
32 nodes; publication can temporarily retain a second JSON tree.

Four integration cases and one unit case cover multi-page exact roundtrip,
offline proposals, path/name collisions, frozen data, actual WAL checkpoint
during review, cancellation, existing output preservation, filter/identity/size
guards and a late missing-file error that discards all staged nodes. Eight GPU
frames cover real controls and 252 captured rules, Cancel, reconnect/inventory
replacement, permission while review is open, invalid parent and retry. The
48-frame general GPU regression covers the changed toolbar. All 151 current
core cases, strict Clippy, formatting and desktop rebuild pass. See
`.verification/multi-node-export-result.json` for final test/check/build evidence.

This advances export parity; native pickers/compositor behavior, actual daemon
import/kernel enforcement and broad resource/package measurement remain unproved.
All other parity gaps, final coherent whole-app UI/UX and post-parity
issue/PR/Little Snitch research stay in the active objective.


## Explicit per-user desktop startup

The pinned UI uses utils/xdg.py::Autostart from preferences and a tray action.
Rustleblower now exposes Startup preferences from the dashboard and tray.
Checkbox changes stay staged until Save; Close changes nothing and Reload
re-reads the effective XDG entry. All discovery and I/O run off the UI thread.
The entry records the active binary/AppImage, absolute socket/history paths
and launch-time permission timeout/fallback, without overriding saved retention.
An inherited parent application's APPIMAGE is ignored unless the executable
is actually inside its APPDIR. Relative endpoint paths resolve at configuration
time so login's working directory does not redirect the server.

User entries override ordered system entries. Disabling writes Hidden=true to
Rustleblower's own user entry; it leaves system/OpenSnitch entries untouched.
Only a managed user entry can be replaced. Fingerprints reject stale saves;
new files publish without clobber and updates use atomic temporary-file replace.
These checks are not a filesystem compare-and-swap against arbitrary writers.
Symlinks, foreign/malformed/oversized entries are preserved with feedback.
Entry reads/writes are bounded to 64 KiB, command 32 KiB and system paths 16.

Three core cases cover precedence, enable/disable/reopen, preserved system and
foreign files, stale changes, symlinks, bad/oversized entries, AppImage identity
and reserved path arguments. Independent desktop-file-validate and actual GIO
launch decode a fixed owned recorder with spaces, quote, dollar, backtick,
backslash, Unicode and percent characters. GIO rejects percent-bearing executable
paths before field-code expansion; only that case uses /usr/bin/env -- with
quoted arguments and no shell. The corrected fixture receives exact arguments.
Six background GPU frames prove actual controls, staged Close, synthetic tray
entry, durable state, real AskRule during save, stale refusal and Reload.
The full suite has 154 passing core cases; the final relative-endpoint change
is covered by the three focused cases and strict Clippy. The general/tray
regressions and rebuilt desktop are indexed in
`.verification/startup-result.json`. Real login scheduling, actual tray menu
D-Bus input for this new entry and compositor proof remain separate.

The remaining parity inventory, final UI/UX consistency pass and post-parity
upstream issue/PR/Little Snitch research remain in the active full objective.


## Native color appearance preferences

Pinned preferences/sections/ui.py and utils/themes/themes.py offer System and
Qt Material themes, with separate Qt-specific density. Following FerryDock's
Kit Settings/SettingPage and Theme::change pattern, Rustleblower now exposes
System/Light/Dark choices, reset and reload. Kit's registered palettes and Base
root projection remain authoritative; no custom renderer or second backend is
introduced. A per-root native appearance observer tracks system mode.

Initialization starts with the current system mode and reads the 4-KiB document
on a background executor. Permission UI can open before that read completes.
One global appearance state and one pending load/save serve all windows. Choices
apply immediately; only persistence runs in the background. A failed/stale save
keeps the session theme with explicit feedback. Reload restores the saved mode;
small malformed/future documents remain unchanged until an explicit choice
repairs them. Oversized, symlink and other filesystem errors are preserved.
The file is separate from popup/history preferences and is now protected from
history export destinations. There are no per-event disk reads.

Three core cases cover defaults and mode roundtrips, unchanged sibling settings,
malformed/future/unsupported mode fallback and explicit recovery, stale writes,
symlink/oversize/invalid-directory refusal. Eleven actual GPU frames cover Kit
Settings controls, dashboard/editor/popup colors, copied unsaved draft fields
and no saved draft, an independent real AskRule across a color switch, stale
file preservation/session-only color, reload/recovery, fresh GPUI-context reopen
and default reset. General/tray regression, full core, the final export-file
protection case and exact build/check evidence are indexed in
`.verification/appearance-result.json`.

The fixed-Light test platform proves System initialization/reset; a real native
system appearance-change callback, broader native/package/resource proof and
whole-app contrast/visual consistency remain unproved. Native custom themes and
density remain separate parity work, as do window restoration/language and the
rest of the full objective. This does not complete the final UI/UX pass.

Appearance proof now records 157 passing core cases plus the final targeted
history-export protection case, strict Clippy and eleven/48/four GPU frames.
The final general/tray flows initialize the same appearance/root observer
composition used by normal application startup.


## Main-window geometry, mode and last page

Pinned events/dialog.py restores STATS_GEOMETRY, STATS_MAXIMIZED and
STATS_LAST_TAB, saving on close/hide. Rustleblower now preserves main-window
restore bounds, windowed/maximized/fullscreen mode and one of its seven pages
in a separate version-1 4-KiB file. Saved rectangles are fitted to up to 16
current displays; disconnected/offscreen monitors fall back to a reachable
rectangle. Wayland's compositor retains authority over actual origin.

A hidden bootstrap creates the permission/dashboard owner immediately. One
background task loads state; after at most 250 ms the UI chooses the default
layout instead. Late state never overwrites an already-shown dashboard. The
bootstrap is removed before opening the final main window, retaining feature
state and the independent permission loop. Initial main-window focus is not
requested while permissions are pending. Native compositor focus under a stalled
read remains separate proof.

A 50-ms UI sample compares fixed-size geometry/page data. One latest-value watch
slot coalesces updates; the writer waits for 500 ms of stability or an explicit
close/quit flush. There is one read/writer task, one pending snapshot and one
latest status, without event retention or per-move filesystem work. Quit shuts
down the backend/pending prompts first, then allows at most 500 ms for layout
flush before UI exit. Corrupt, unsupported, symlink, oversized and externally
changed files remain preserved; an error stops automatic persistence until a
new process reloads the file. This is not an arbitrary-writer filesystem CAS.

Three core cases cover 21 page/mode combinations, sibling files, finite bounds,
negative/disconnected/small monitor fitting and invalid/stale files. The full
core suite has 160 passing cases, including history-export protection of the
new filename. Eleven GPU frames use the production bootstrap/writer and actual
navigation/permission/Quit input. A 25-update geometry burst produces one write;
warm/cold restoration, independent hidden/loading requests, a held load-result
delivery with default timeout, stale-save stop and quit fallback/flush pass.
Geometry requests and the compositor callback shim are synthetic here; no stalled
kernel filesystem read is claimed. General 48-frame and tray four-frame
regressions pass after the shared lifecycle change.

Two owned native KDE Wayland runs add real resize/maximize, exact-file handoff
to a fresh process, restored maximized Nodes page and original unmaximized size.
Both observe 430 fixture events, zero policy commands and full scoped cleanup.
Fractional compositor geometry is compared independently of GPUI's client-area
restore bounds. Native fullscreen and X11 restoration, monitor-disconnect/native
focus variants, packaged/RSS proof and other layout/filter state remain pending.
See `.verification/window-state-result.json` for exact artifacts and binary hashes.
The final whole-app UI/UX pass and all other parity/research scope stay active.

### Remembered history presentation

The pinned events/dialog.py restores/saves general search text, action filtering
and table header state. History now restores search/action, applied typed
conditions, header sort and per-field widths alongside its existing compact/table,
column and refresh choices. This is the native equivalent of saved history
presentation; other page layouts and splitters remain separate work.

`history/settings.rs` version 3 reads versions 1/2 with defaults, preserves
independent field changes under one writer lock and caps its file at 16 KiB.
Search is 256 bytes, conditions are at most eight 256-byte values and widths
are at most nineteen unique finite 40–2000 px entries. Saved queries deliberately
omit row selection, keyset cursors, arrival ceilings and earlier-page offsets:
startup follows the first current matching page. Hidden-column widths stay
associated with fields when the visible column set changes.

One history presentation worker and one pending watch slot wait for 500 ms of
stability. UI input never writes files. Close/Quit request a flush; Quit shuts
down backend/permissions first and shares its existing 500-ms deadline between
window and history writers. Column/refresh saves merge only their fields.
Raw-file hash guards and private-file publication with file/directory fsync
preserve externally changed files; they are not arbitrary-writer filesystem CAS.
A failed automatic save stops further writes and explains session-only state.
Malformed/unsupported/oversized/symlink settings at startup use defaults with an
error and disable writes; they cannot prevent the RPC server from starting.
Explicit file repair/removal plus restart restores saving. Do not repair user
files as part of verification.

Four core cases prove migration/merge, maximum bounded state/validation, stale
and symlink preservation, and actual Ping/AskRule/reply with three damaged-file
variants. The full isolated-network core suite has 164 passing tests. The
sixteen-frame history-tools GPU journey adds actual Kit column-edge pointer
dragging, visible/control restoration across fresh backends/GUI contexts, exact
72 filtered rows, immediate Quit flush, stale-save feedback, independent real
permission input and explicit fixture recovery. General 48-frame, tray four-frame
and window-state eleven-frame regression journeys pass after the Quit change.
An owned native KDE filter run records exactly UID Equals 9999 in the saved file,
430 history rows, zero policy commands and complete cleanup. Native column resize,
cold history restoration, other pages, current packages/resource measurements and
the required final whole-app visual pass remain unproved or pending.

See `.verification/history-presentation-result.json` for artifacts and exact
desktop/source hashes. The full parity and post-parity research objective remains
active; this section does not redefine its completion criteria.

### Subscription URL diagnostics

The pinned `list_subscriptions/ui/workers/url_test_worker.py` and subscription
dialog controller provide a pre-save URL test. Rustleblower now exposes Test URL
and Cancel URL test in the fixed add/edit form footer. It validates only the
draft URL and selected format, so users can test before completing name/filename.
Save remains explicit and does not require a successful diagnostic. No probe
publishes files, changes subscriptions/logs or issues daemon policy commands.

HEAD uses a three-second deadline, with GET fallback only after 403/405. Hosts
format samples GET for up to five seconds; a single GET serves both fallback
and sampling. Other formats remain reachability-only. The task has a twelve-
second total limit, five-redirect cap, 2048-byte credential/fragment-free HTTP(S)
targets, saved user agent, ordinary TLS verification and a 200-line/64-KiB sample.
Stream fragments are assembled before UTF-8/sample inspection. The pinned pure
hosts heuristic is retained, including its permissive <=10 non-comment-line
case; the UI explicitly says short samples are inconclusive. Larger samples
require 60% recognizable entries. This is separate from full download validation.

The existing subscription runtime admits one probe alongside its single normal
download. Requests/replies remain bounded; the probe owns one cancellation token
and does no filesystem or SQLite writes. Drop/cancel releases its stream; a
cancelled task is reaped before immediate replacement so the new test does not
receive a stale busy response. Concurrent uncancelled tests are refused. URL/
format changes discard prior results; Save/Cancel/form destruction, page leave,
dashboard close and worker shutdown cancel only that diagnostic. Permission
RPCs, catalogue/settings requests and normal downloads continue independently.

Six core cases cover all those bounds plus HEAD/GET/status/fallback/redirects,
chunked split lines, large/many-line samples, short/bad hosts, non-host formats,
broken streams, TLS protocol failure, HEAD timeout, drop/restart/shutdown and real
permission replies. A held normal download retains its previous published bytes
through successful/cancelled probes. The full core suite passes 170 tests; the
final cancellation-slot cleanup also passes all six focused cases. Nine actual
Rust sample results match the unmodified pinned pure Python helper through the
new `verify-url-test-heuristic.py` adapter, without executing Qt or a plugin.

Eleven GPU frames cover actual URL/format/Test/Cancel/navigation/form/Save input,
redirect/sample results, held-HTTP permission input, stale results and zero writes
until explicit Save. The 56-frame subscription regression and general 48/four-
frame tray regressions exercise the shared worker/form/lifecycle paths. A native
KDE run proves keyboard Test URL access on the ordinary-size form, visible scheme
refusal, zero subscription/log/policy changes, 430 events and owned cleanup.
Actual native successful HTTP/cancellation, trusted/untrusted certificate variants,
compression, broad resource/package proof and final UX remain separate.

Source review also corrects the scope map: metadata archives are our migration-
recovery mechanism, not an upstream page. Keep their data and reimport recovery;
an archive-management UI is an optional Rustleblower improvement, not an invented
gate for source parity. Legacy default-unit coupling, notifications, the other
documented source features, all required final UX work and post-parity research
remain in scope. See `.verification/subscription-url-test-result.json` for the
exact implementation/proof boundary and retained failed attempts.

### Independent subscription count and unit inheritance

Individual subscription and global-default editors now retain counts and units
separately, following the pinned pure Python models. Either part can inherit;
the form previews effective seconds/bytes. Existing numeric native values remain
absolute. Version-2 native backups preserve the new optional fields and still
read version 1. OpenSnitch action export preserves null inherited fields and
canonical units. Previously normalized old imports need the original source
file reimported to recover choices that were already lost.

Default changes validate every retained dependent subscription and update normal
schedules atomically with settings and logs. Invalid dependent values, late SQL
errors and stale editor revisions refuse the save without partial changes.
Failure backoff and existing active-download cancellation cooldown are retained.
Bounds and one-at-a-time I/O remain unchanged; no new dependencies or backend.

Validation passes 174 core cases, strict Clippy, twelve focused GPU frames and
56/11/48/four subscription/URL/general/tray regression frames. Fifteen actual
Rust effective-limit tuples match the exact pinned Python models. Three native
journeys verify keyboard unit-only save, URL validation and source import with
the same rebuilt desktop binary, 430 owned events each, zero policy commands
and complete cleanup. Read `.verification/subscription-inheritance-result.json`
for evidence and hashes. This does not prove live-daemon enforcement, native
default/stale/reopen variants, real-time scheduling, packages or resource budgets.
Independent bulk count/unit controls remain source-backed parity work, alongside
the other outstanding features and final UX. The user requested pausing the goal
after this slice; do not start the next feature.


### Bulk subscription count/unit controls and frozen defaults

Bulk subscription forms now use the existing Kit count/unit controls. Checked
limits can inherit either or both parts; unchecked limits preserve their exact
existing overrides. Raw before/after choices and effective seconds/bytes are
shown beside the captured default counts/units. The review captures one bounded
settings snapshot/revision and application checks it in the SQLite transaction
before its first write. Any default change requires an explicit fresh review.
There are no extra workers, unbounded lists, or implicit daemon policy commands.

Source review distinguishes the pinned controller's pair replacement from its
view's zero-count-clears-both serialization. The empty-plus-Inherit choice
preserves that outcome; unit-only bulk choices extend the native consistency of
our existing model and individual editor. Do not claim that the pinned Qt bulk
view itself supports unit-only input.

Two core cases cover structured/untouched legacy values, domain/count/overflow
refusal, changed/tampered defaults, late SQL/log rollback, exact whole-batch apply
and reopen after default count/unit changes. The full suite passes 176 cases.
The expanded 61-frame subscription journey covers actual selectors, Back,
effective review, default changes, refusal/rebuild and stored independent limits,
alongside its existing HTTP, permission, rule, collection and transfer paths.
Evidence and precise remaining limits are in
`.verification/subscription-bulk-units-result.json`. Keep failed attempts.
Native bulk-unit gestures, subscription notification parity, crash recovery,
resource testing and the full-app UX phase remain pending. The full goal is
active again; this slice does not establish overall feature parity.


### Subscription result notifications — production integration

The source-backed contract and native differences are recorded in
[subscription-notifications-plan.md](subscription-notifications-plan.md).
The Blocklists editor saves optional success/error text into retained source
metadata with transaction/budget/revision guards. Completed normal downloads
use the existing eight-item mailbox and one D-Bus worker; each current notice
carries an Alerts or Blocklists action target. HTTP304 succeeds, failed checks
notify, stale/cancelled work and URL tests remain silent. No new renderer,
per-notice thread or prompt-path dependency was introduced.

Current proof: production `cargo check --locked` passes, and the private owned
D-Bus integration test passes with subscription title/body/actions, stale-kind
action rejection and separate screen targets in addition to existing owner,
mute, timeout, independent Ping/AskRule and cleanup assertions. Evidence:
`.verification/subscription-notifications-dbus2.log`. The first sandboxed run
could not bind its private bus and is retained as an environment refusal, not
feature proof. Config/worker and actual GPUI editor/restoration proof are being
run separately by the verification agent; the overall goal remains open.


Combined integration checkpoint: strict
`clippy --all-targets --all-features --locked -- -D warnings` passes after the
notification, appearance, history-search, firewall-helper and initial remote
transport integration (`.verification/parallel-slices-clippy2.log`). The current
headless-feature build also passes the private D-Bus integration again
(`.verification/subscription-notifications-dbus-current.log`,5.07 seconds).
The four focused subscription notification tests pass in an owned loopback
namespace, including HTTP200/304/404 and stale/cancel/probe silence
(`.verification/subscription-notifications-core.log` and its network report).

The first strict lint run and UI attempts are retained with their causes:
history style/new-harness conversion errors; density control clipping under the
fixed footer; missing notification Ctrl+Enter dispatch and a private-method
visibility mistake. The production controls/dispatch and harness observations
were corrected. Exact font/rem checks remain strict; rendered button geometry
allows only one physical pixel of pinned-layout rounding, recorded with the
actual device scale. Expanded density/palette, notification and guided-firewall
journeys remain pending until their successful result files are recorded.
The combined checkpoint source hashes are
`.verification/parallel-slices-source.json`; later edits require fresh proof.


Subscription notification GPU proof now passes nine frames under
`.verification/headless/20261017T022211Z-996e1089` (20.917 seconds, real GPU,
owned process exit and scoped socket cleanup). It drives saved custom text,
Back, stale source revision refusal, reload, Ctrl+Enter, disable-clear and
separate hidden Alerts/Blocklists callbacks, while answering a real AskRule
through input. Root visually inspected the editor screenshot: labels, values
and actions are readable without overlap. A later copy-only cleanup removes
HTTP/source-format jargon from the product flow. The source/protocol semantics
and measured behavior stay in development documentation. Physical desktop
notification banners/clicks remain outside this owned proof.

# OpenSnitch UI parity plan

The project objective is full native UI parity while keeping the existing daemon
and its policy ownership. At the user's request, autonomous work stops after the
typed daemon preferences slice; the remaining inventory is a backlog for a future
resume. A working screen or a successful build alone does not
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
| Rule workflows | Duplicate, multi-select/bulk actions, import/export JSON and clipboard, apply to nodes, create from event | Duplicate, cross-page selection, bulk actions, file/folder/clipboard interchange, and reviewed application/destination drafts from history implemented for current node. Core/background GPU verified; per-node catalogue/editor targeting now available through Nodes; cross-node bulk copying and native picker/compositor proof pending |
| Prompt options | Reject, duration/target choice, advanced multi-field matching, defaults and position, disable popups | Reject, nine duration presets, derived command/domain/network/checksum targets, AND constraints, priority/logging, live details, saved basic/advanced defaults, UI timeout/fallback controls and disabling popups implemented; core/background GPU verified. Saved X11 initial placement has native geometry/input proof for Center/four corners; native Wayland retains compositor placement with the selector disabled. Queued requests retain captured settings; unavailable default fields open review feedback. Lasting replies await daemon observation; explicit inventory refresh available. Interaction pauses the ordinary countdown with a visible finite RPC limit; pointer/keyboard GPU and Wayland/X11 input proof cover it. Checksum-change warnings and a separate selected/all update review are implemented for enabled exact executable/hash AND rules, with core/GPU and native review-window input proof. Native update controls and broader input proof remain pending |
| Connection details | Source/destination, PID/UID, command, process tree, checksums, copy and rule creation | Historical metadata/arguments/matched-rule snapshot, copy, current-rule lookup and reviewed rule creation implemented with bounded disk snapshots. Core/GPU verified; UID-name resolution, live process/tree/checksums and native input proof pending |
| Statistics | Hosts/apps/addresses/ports/users/protocols, counts, charts, drilldown filters, reset/export | Implemented with bounded persistent daemon-counter cache, node/search/sort controls, Kit top-ten chart, frozen paging, exact history drilldown, local-baseline reset and CSV/JSON/clipboard export. Core/GPU and native Wayland/X11 open/search/persistence verified; native reset/chart/export/pickers and long-duration RSS remain pending |
| Nodes | Multiple nodes, detail/resources, interception controls, per-node configuration and rule targeting | Independent named Unix sessions, bounded cached metadata, per-node rules, confirmed interception/configuration and scoped refresh implemented. Core/GPU and native read/search/persistence verified; live resources, peer identity/TCP/TLS, node removal/export and cross-node bulk copying remain pending |
| Daemon preferences | Default action/duration, logging, monitors, rule paths, network/device options | 29 typed fields in five groups, validated changed-field patching, bounded before/after review and complete JSON implemented. Unknown/unreported values and exact decimal numbers preserved. Core, focused/full GPU and native default-action edit/review/save verified; apply-to-all-nodes, exhaustive field variants and actual TLS connection proof remain pending |
| Remote transports | TCP, TLS/mutual TLS, certificates, connection/auth configuration | Pending; revisit local-only authority compatibility exception before TCP |
| System firewall | Profiles, chains, ordered rules, full expression editor, enable/disable and reload acknowledgements | Pending |
| Live sockets/processes | Netstat filters and polling; daemon tasks for sockets/process details; stop task lifecycle | Pending |
| Alerts | Persist/display/filter/copy/export text and typed alerts, rule alert configuration | Pending |
| Desktop | Tray, desktop notifications, autostart, window restoration, appearance/density/language/shortcuts | Pending |
| History preferences | Retention/database settings, event columns, temporary-rule filters, refresh intervals | Compact/table mode and chosen columns persist in a bounded document. Retention/database UI, temporary-rule filters, refresh settings, and persisted widths/sort/filter presets remain pending |
| Plugins and extensibility | Inventory upstream plugin features and supply native equivalents/integration where applicable | Pending |
| 1.8 compatibility | Compare latest UI and wire protocol; add supported features and capability feedback | Pending |
| Upstream ideas | Review issues/PRs after parity; document concrete value and implementation evidence | Pending |
| Little Snitch ideas | Review official product/docs, then adapt useful workflows to the existing daemon | Pending |

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

The user requested stopping after this feature. The remaining full-port features
and subsequent OpenSnitch issues/PRs and Little Snitch research are explicitly
unfinished and will not start automatically after this slice.

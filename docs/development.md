# Development

FerryDock at `/home/maximep/projects/ferrydock` is the architectural reference.
Prefer the pinned GPUI Kit controls, keep feature state and views together, and
keep I/O off the UI thread.

`app.rs` composes the dashboard; `app/history_view.rs` owns search, bounded pages,
stale-query rejection, and virtualized rendering. `app/prompt_window.rs` owns the
separate permission window. The window close action minimizes rather than
stopping the server. Quit is explicit.

`backend.rs` owns the small Tokio runtime and Unix-socket gRPC server. Each
accepted transport gets a connection ID. Subscribe installs a local daemon
session without changing its configuration. Replacing a session invalidates its
pending prompts; losing its Notifications stream disconnects it. Ping transports
statistics/events; AskRule is a unary RPC held until a decision or timeout;
Notifications is a bidirectional stream with daemon replies inbound and UI
commands outbound. This slice keeps that stream open and reports incoming errors;
application-rule commands use it and wait for matching daemon acknowledgements.
PostAlert currently exposes the
latest alert in the dashboard rather than implementing alert history or plugins.

Ping may precede Subscribe because the real daemon starts subscription
asynchronously. Each transport can hold at most 32 early report batches; their
bounded display records reserve permits from the same 8 MiB budget as queued
writes. Subscribe drains them under the registered daemon name, retaining normal
deduplication and ordering. Disconnect releases pending records and counts their
loss; an already subscribed, replaced transport cannot buffer as a new connection.
Neither Ping nor Subscribe waits for disk or for the other RPC.

`prompts.rs` owns bounded live requests and oneshot replies. The UI only borrows
snapshots, and stale/expired IDs cannot authorize later requests. Dropping a
cancelled RPC removes its prompt. Decisions use OpenSnitch's composite operator
format. `prompts/choice.rs` derives bounded target/constraint options from the
immutable request and validates every choice again before resolving its ID.
Allow/deny/reject and nine duration presets are supported. Conditions are flat
AND groups with both protobuf list and JSON representations for daemon 1.7.2.
Relative or `/proc` commands and checksum targets retain an executable guard.
Timeout, session replacement, overload, and shutdown always use a once-only
fallback regardless of selected options. No lock spans an await or database call.
Requests are capped at 64 KiB encoded and 256 KiB retained allocations, including
string/vector/map capacities. Live detail rows are virtualized and disappear
with the request; history does not retain environment/checksum/process-tree data.

`preferences.rs` owns a versioned UI popup preferences document beside the chosen
database (`<database>.preferences.json`). Reads are capped at 4 KiB; missing files
use defaults while malformed, oversized or unsupported versions fail startup
explicitly before socket binding. Saves validate enumerated choices, sync a
private temporary file and atomically replace the document. A writer mutex
serializes saves; filesystem work does not hold the separate snapshot RwLock.
The RPC path reads only the bounded in-memory snapshot. File replacement failure
preserves the last good in-memory settings. This is independent of SQLite locks.
Files now write version 3 and read legacy versions 1–2 with optional new fields
defaulted. Earlier binaries reject version 3 instead of silently ignoring saved
fallback or condition overrides. Unknown versions and wrong optional-field types
are rejected. Four enumerated default conditions are accepted with no duplicates.

`app/preferences_view.rs` uses pinned Kit Select/Checkbox/Button controls and
saves on the background executor. It applies only to new requests: each admitted
prompt captures its initial duration/target/priority/logging, reported advanced
conditions, expanded-options setting, effective fallback and deadline. An unavailable
target falls back to the connection match. Disabling popups immediately replies
once using the configured fallback for new AskRule calls, without creating a
prompt or counting overload; already pending requests remain valid. This mode
has a dashboard reminder and a distinct `popups-disabled` history reason.
The timeout field retains at most three ASCII digits and validates 1–110 on save.
Blank timeout / Use launch setting inherit the CLI defaults; explicit saved
values override them for new requests. AskRule caps its captured timeout by the
daemon RPC deadline minus the existing 100 ms reply margin. Shutdown, stream loss
and resubscription use each pending request's captured fallback, rather than the
latest preferences or CLI base. The prompt subtitle displays its captured action.

Interaction pauses the ordinary UI countdown, matching installed 1.7.2
`PromptDialog._stop_countdown` / `_timeout_worker`; that daemon client uses a
120-second `AskRule` context (`daemon/ui/client.go`). Admission captures both
the ordinary deadline and a hard deadline: the smaller of RPC metadata / 120
seconds, minus a 100 ms reply margin. Missing or longer RPC metadata retains
that finite upper bound. Opening initially expanded options pauses the countdown
only when the popup renders that request. Pointer capture and a per-window GPUI
keystroke interceptor pause the displayed ID without consuming events. The
interceptor covers Kit action bindings such as Tab; element key listeners alone
miss them. Input in another window cannot pause a permission request.

`Prompts::review` updates only a live ID before its current deadline and cannot
revive expired/answered/replaced requests. Repeated input never renews the hard
limit. Each of at most 32 pending entries has one watch deadline, with no queued
input history. `wait_for_expiry` rechecks the actual deadline under the prompt
mutex before returning, so review races cannot trigger the old timer. It runs
in the existing RPC future without UI, telemetry, SQLite or per-event tasks.
Cancellation/reconnect/shutdown keep the captured once-only fallback. Review
status displays the remaining hard request limit; minimizing does not remove it.

Saved IP/port/UID/checksum conditions are resolved from the immutable admitted
connection; checksum selection prefers reported MD5, then SHA1. A missing field
is omitted from the generated conditions and listed in a visible warning with
options expanded for review. The summary shows the additional-condition count.
No automatic fallback acquires lasting duration or these optional conditions.

`rules/checksums.rs` and `app/prompt_checksums.rs` implement the installed
`dialogs/prompt/_checksums.py` / `_display_checksums_warning` workflow with native
Kit controls and explicit daemon acknowledgements. Both MD5 and SHA1 are compared
when the admitted request reports valid hexadecimal values. Only enabled, flat
AND rules with an exact simple executable-path condition and simple hash operands
are candidates. Case sensitivity follows each simple operator for ASCII path/hash
values. Regex, list-file, nested and unreported hash conditions are not rewritten.
The comparison uses confirmed `application_rules`, never unacknowledged proposals.

A popup probe reads on the background pool and retains only mismatch presence or
an availability error. One global try-acquired semaphore bounds concurrent read/
prepare work across probes and the single checksum-review window. Missing inventory
or a busy/erroring reader does not gate permission controls or deadlines. Read-only
connections use a 512 KiB suggested cache, mmap=0, query_only and 250 ms busy timeout.
Scans consider at most 10,000 catalogue records; visible/work pages retain at most
16 rules and 512 KiB including copied differences. Rule/difference lists are
virtualized. Review remains open after the permission is answered.

Selected/all updates require explicit confirmation. Each rule is reread by name
and compared with its reviewed protobuf before preparing replacements. A changed,
removed, disabled or no-longer-mismatching rule stops the workflow for re-review.
Preparation replaces reported simple hashes and rebuilds the protocol's JSON list,
preserving unrelated conditions and all rule metadata. The existing bounded Control
sends one CHANGE_RULE at a time; only its matching OK and persisted cache update
advance progress. Error/timeout/disconnect stops unstarted work and preserves the
existing uncertainty feedback. All-page iteration retains one page and a name
cursor, caps attempts at 10,000 and does not promise a frozen daemon snapshot.
Concurrent external daemon edits remain subject to the protocol's last-writer
semantics; the local reread cannot supply an absent daemon-side compare-and-swap.

Stop clears unstarted work and waits for any sent command's result. A running
review minimizes on native close; its 100 ms task polls progress independently
of rendering. Once idle it can close normally. A process-global weak window handle
prevents multiple simultaneous checksum dialogs; a different request asks the user
to finish/close the existing review. Successful updates invalidate popup probes.
The focused `--feature checksums` GPU flow covers two hashes, pages, confirmation/
cancel, held ACK without optimistic cache changes, an independent permission reply,
stop, failed confirmation, explicit fresh inventory and all remaining pages.
Native `--feature checksums` proves opening the separate review by keyboard and
returning to answer the prompt without mutation; native update controls remain
separate from that proof.

`prompts/placement.rs` computes five initial positions with a 16-logical-pixel
margin and clamps the requested content size to the available rectangle. Its
core tests cover negative monitor origins, small workspaces and invalid geometry.
`app/placement.rs` uses the actual raw display handle to enable X11 placement;
native Wayland retains compositor placement with the selector disabled.
The native integration uses the existing GPUI backend and x11rb for desktop
metadata/window requests. A background task caches one primary RandR monitor
intersected with the current EWMH work area, refreshing every five seconds.
Window opening reads only the cached rectangle. Placement never waits on daemon
traffic, history or the main-window visibility.

After the popup's first frame, a one-slot watch channel retains only the latest
move request. The background task makes at most 200 mapping/manager checks,
spaced 25 ms apart, for a mapped window and ICCCM NormalState. It validates the
PID, reads fresh workspace/frame extents and
sends a managed `_NET_MOVERESIZE_WINDOW` request. It checks the request before
closing the short-lived X11 connection; flushing alone did not provide reliable
delivery. No UI-state lock spans native I/O. A newer request replaces a pending one, and a
destroyed or no-longer-owned window is ignored. Native frame extents keep the
entire outer window inside the workspace. Corner-to-center queue transitions
replace the previous popup, preserving captured per-request preferences.

For opt-in graphics/placement diagnostics, set
`RUSTLEBLOWER_GRAPHICS_DIAGNOSTICS=1` when launching an owned development instance.
It prints graphical warning/error messages and placement request outcomes to
stderr. It is off by default and does not log traffic payloads.

The daemon's `AskRule` returns its parsed reply without a notification ACK.
Lasting user replies are therefore proposed asynchronously to a separate
`prompt_rules` table, sharing catalogue count/byte bounds with at most 128 pending
entries. Rule pages label them awaiting observation and disable mutations.
Only a daemon-reported Ping with the matching rule name promotes a proposal;
the local decision-history record cannot confirm it. The reported metadata
replaces the proposal. A fresh Subscribe inventory clears unobserved proposals
for that node. Exports include them; bulk mutations and current-rule editing
require confirmed entries. Queue/storage failure never delays AskRule.

`history.rs` owns SQLite writes on one serial worker thread. Deduplication hashes
the original event with sorted, length-framed environment/checksum maps, scoped
to the daemon name. This avoids replay duplicates caused by protobuf HashMap
iteration order. Environment/checksum values contribute only to the fingerprint;
they are not persisted as details. Different events can have the same summary.
Queries materialize one
page before releasing the database; no UI cursor holds a read transaction open.
Limits and failure semantics are documented in the README. Loss counters are
per application run; the database is not a lossless audit log.
Loss counters distinguish queue pressure, event limits, storage failure, and
connection loss. Each committed 128-record chunk updates the page revision;
failed writes count only the uncommitted remainder. A maintenance error does not
count committed rows as lost. Successful maintenance clears the active database
error while the warning retains any historical loss for the current app run.

`history/details.rs` encodes a bounded historical snapshot: protocol, source and
destination, UID/PID, executable, cwd, arguments, daemon time and matched-rule
metadata. Historical conditions are not copied: opening a current rule reads its
full catalogue entry separately. Environments, checksums and process trees remain
excluded; live process inspection is a separate pending feature. Paths/cwd are
capped at 4096 bytes, addresses 64, host 1024, arguments 64 entries / 8 KiB total /
1 KiB each, and the complete detail protobuf 24 KiB. Flags identify shortened
fields; drafts cannot use an incomplete executable or destination automatically.
Queue permits are reserved incrementally before retaining each normalized record,
and partial acquisition rolls back when an entire batch cannot fit.

Schema 2 adds `event_details` with a payload-size check and cascading deletion.
Both the event summary and its detail are inserted in one transaction, with
detail allocation included in retention accounting. Existing schema-1 summaries
remain readable and explicitly lack a snapshot; no historical metadata is
invented during migration. Schema 3 adds the separate popup proposal catalogue;
schema creation and the version change commit atomically. Older binaries reject
newer schema versions. Retention scans at
most 512 oldest sizes and removes only the prefix needed for the disk budget,
avoiding complete eviction of a small history when per-event details are large.

`app/connection_details.rs` uses Kit DescriptionList and TabBar with one detail
window and on-demand reads. History-to-rule requests use a four-slot UI mailbox
with opening results, node/session/readiness checks, and the existing single rule
window. New/renamed rule names are checked against the current catalogue on a
background reader before sending a change. The quick new-rule form uses the same
check. Imports retain their explicit Replace option. The protocol has no remote
compare-and-swap name reservation; these are checks against the confirmed cache.

## Checks

Use `python scripts/cargo.py` in place of Cargo to prepare the local dependency
patch automatically. To use plain Cargo or an IDE, first run
`python scripts/prepare_dependencies.py`. Upstream source stays under ignored
`.deps/`; only our small diff is maintained in `patches/`. Preparation reuses
Cargo's verified crate cache and otherwise downloads the pinned archive. Add
`--offline` to require cached input. See
[transport-compatibility.md](transport-compatibility.md) for details.

```sh
python scripts/cargo.py fmt --check
python scripts/cargo.py test --no-default-features --locked
python scripts/cargo.py clippy --all-targets --locked -- -D warnings
python scripts/cargo.py build --locked
```

Keep the desktop build after `test --no-default-features`: Cargo uses the same
top-level binary path for both feature sets. Native helpers snapshot the built
executable and verify its identity. The doctor excludes the cfg(test)-only
headless test module from freshness checks because Cargo does not compile it
into that binary. Native helper guard regressions run with
`python -m unittest discover -s tests -p test_native_desktop.py`.

To prove compositor placement, use
`python scripts/verify-native.py run --feature placement --backend x11 --position bottom-right`.
Repeat for center/top-right/top-left/bottom-left. Each run reads actual owned
window geometry, native frame extents, RandR primary monitor and EWMH workspace;
it never moves the window from the verifier. Geometry must match within two
logical pixels (at least three physical pixels for rounding), then an input
decision must be received and persisted. Evidence records all coordinates.
Native Wayland prompt input is verified separately; it does not prove corner
placement. X11 title matching reads UTF-8 EWMH names after a bounded PID lookup,
never reuses a previous window after a failed lookup, and waits for actual focus.

`rules.rs` owns the disk catalogue query/serialization and bounded command
coordinator; `app/rules_view.rs` owns its Kit controls, filtering, paging, draft
and selection state. Full protobuf operators remain intact during action or
enabled-state changes. Rule names cannot contain `/` or NUL because the daemon
uses them as filenames. Subscribe replaces a node's snapshot transactionally.
Command edits are applied to the local catalogue only after a matching OK reply
from the current session. Negative replies, reconnects, and ten-second timeouts
resolve the pending operation explicitly. A failed local catalogue update after
an acknowledgement invalidates its edit readiness until resubscription.

Rule inventories and catalogue updates reserve the shared history-worker memory
budget. Inventories are capped at 10,000 rules / 2 MiB of protobuf data; individual
rules are capped at 64 KiB encoded and 256 KiB of retained allocations. Each rule
query returns at most 200 rows. Pending commands are capped at 16 and the outbound
stream queue at 8. No notification operation waits on storage or the UI thread.
`tests/rules.rs` verifies catalogue paging/search/duration filters, durable storage,
acknowledgement before mutation, rejection, preserved operators, enable/disable,
delete, and stale-session replies. The background UI flow drives the first rule
journeys against an isolated gRPC daemon fixture and captures the error state.

`rules/transfer.rs` provides OpenSnitch JSON file/directory/clipboard interchange.
Import accepts RFC3339/numeric dates, omitted protobuf flags and legacy list data.
It borrows raw array elements before allocating one bounded JSON object, then
checks count, encoded size, retained memory and duplicate names incrementally.
The document limit is 8 MiB, individual JSON objects 512 KiB, imported/reviewed
rules 10,000 / 2 MiB encoded / 8 MiB retained. A directory scan is capped at
20,000 entries and 10,000 JSON files. Those limits are checked off the UI thread.

Exports/reviews open an independent read-only SQLite connection on the background
executor with a 2 MiB suggested cache, no mmap and a 250 ms busy timeout. One
transaction covers keyset pages, keeping the snapshot consistent while history
continues writing. JSON files use a temporary file and atomic replacement;
directory exports stage a fresh child directory and publish it only on success.
They never overwrite the active history database. Directory filenames preserve
distinct rule names with/without `.json`; long names use a hash while the JSON
retains the original name. The main window allows only one rule workflow window.

`app/rule_transfer.rs` owns review, daemon-confirmation progress, stop and retry.
Only the remaining rules are retained after failure/stop. Cross-page selection
stores names; payloads are loaded from the disk snapshot before bulk review.
Bulk applies one command at a time, preserving bounded stream/pending capacities.
The background GPU flow verifies 201 selected names across pages, all 229 remaining
rules, partial import failure/retry, explicit replacement and a permission popup
while a bulk acknowledgement is held. Native file-picker and compositor input
proof remain separate.

The daemon's `Loader.Replace` updates memory before saving, and `Loader.Delete`
removes memory before deleting its file. Therefore an ERROR is not proof that
policy stayed unchanged. Cache updates still require OK, but any command error
sets persistent `rules_uncertain` feedback until a fresh inventory is saved.
Retry remains an explicit user action; automatic reconnection does not interrupt
pending permission requests merely to refresh a failed rule command.

**Refresh connection** explicitly ends the current Notifications stream with
UNAVAILABLE; daemon 1.7.2 treats receive errors as a disconnect and resubscribes
with a fresh inventory. The server/socket stay in place. Refresh refuses pending
permission decisions or commands, verifies the session, and does not send daemon
STOP or modify daemon configuration. Old catalogue values remain read-only
until the new inventory is stored. The protocol test asserts socket inode
preservation, refusal while work is pending, and replacement on resubscription.

Core tests need permission to bind disposable local Unix sockets. They exercise
the production server through generated gRPC clients, without contacting the
system daemon. SQLite tests cover reopen/deduplication, filters, pagination, and
retention. The desktop can be excluded with `--no-default-features`; such a binary
requires `--headless` and automatically resolves unanswered prompts by timeout.

For native verification, run the README demo on an owned socket/database. Check
both decisions, keyboard navigation, countdown, minimized-window behavior,
search/filters, Older/Latest, and history after restart. Keep screenshots and
local reports under ignored `.verification/`. A build or gRPC test does not prove
native focus or compositor behavior. The compositor may refuse activation;
timeouts still resolve without UI delivery.

The fake daemon accepts `--burst-batches`, `--batch-size`, `--prompts`, `--seconds`,
and `--report`. These generate only synthetic events. Measure RSS and latency
with representative loads before claiming a performance improvement over Qt.

Builds require the GPUI Linux development packages
(including Wayland/X11, font, and graphics dependencies). Cargo locks dependency
versions. Do not mix GPUI backends or copy Docker-specific dependencies from
FerryDock.

## Protocol bindings

`proto/rustleblower.proto` is the MIT-licensed Rustleblower interoperability
contract. It preserves daemon-compatible wire tags, scalar types, enum values,
package and RPC paths. `src/protocol.rs` is generated from that local schema with
the pinned MIT-licensed Prost/Tonic tools; it contains no imported upstream
implementation or explanatory comments. The initially imported bindings were
replaced before the first published commit. The daemon itself remains a separate
GPL-licensed program and is not bundled.

Normal builds compile the checked-in file without `protoc`. Intentional updates
use the separate locked generator:

```sh
cargo run --manifest-path tools/protocol-generator/Cargo.toml --locked
```

See `proto/README.md`. Run core tests and independent installed Python-client
compatibility checks after changes. Rustfmt skips the generated module.

## Project verification skill

Read `.agent/skills/verify-rustleblower/SKILL.md` before UI proof. Run
`python scripts/verify-headless.py` for real GPU rendering and input without
using the desktop. It preserves twelve PNGs, actions, and result/cleanup reports.
Foreground checks use `python scripts/verify-native.py doctor` followed by
`run --feature history` or `run --feature prompts`. The native adapter is derived
from FerryDock's KDE/Wayland and X11 helper; its license is retained in `LICENSES/`.
A foreground run takes focus and owns its input daemon; use the background flow
while the user works. The feature map distinguishes proof from untested entries.

## Unix-socket authority compatibility

OpenSnitch 1.7.2's gRPC-Go 1.32.0 custom dialer passes a bare Unix path as
`:authority`. Stock h2 rejects it before tonic receives Ping or Subscribe.
Rustleblower carries a feature-gated h2 0.4.20 diff in `patches/h2-opensnitch.patch`; it
normalizes path-shaped invalid authorities to localhost only for the five exact
UI RPCs. The app remains Unix-socket-only. Valid hosts and other request paths
retain normal validation. There is no proxy process or extra retained buffer.

The Python fixture now uses the natural C-core authority (a percent-encoded socket
path). Do not reintroduce a localhost override into routine verification. The raw
HTTP/2 regression test covers both representations and unrelated rejected hosts/paths.
When updating h2, review [transport-compatibility.md](transport-compatibility.md)
and rerun the regression.


## History filters, columns and export

Installed 1.7.2 `dialogs/stats.py` defines fourteen general columns (time, node,
action, source port/IP, destination IP/host/port, protocol, UID/PID, process,
command line and rule) and CSV export. Rustleblower exposes those plus duration,
logging, priority, reason and snapshot-shortening status. `history/query.rs` owns
the typed field/comparison allowlist and bound SQL parameters. At most eight
256-byte filters combine with the quick search/action filter; wildcard characters
are literal. Numeric/time validation rejects invalid ports and times. Missing
legacy fields remain SQL NULL / empty text; UID zero remains a reported number.

Schema 4 adds bounded JSON metadata beside the existing summaries/detail blobs.
New ingestion derives it before snapshot truncation and charges its allocations
to the existing shared queue/retention accounting. Source/destination IPs are
capped at 64 bytes, host/rule at 1 KiB and command text at 8 KiB / 64 arguments;
only the fixed known fields are serialized. No environment/checksum/tree is added.
A NULL metadata marker identifies legacy rows. The writer backfills at most 32
24-KiB snapshots between commands or on a 10 ms idle timeout, with an increasing
ID cursor and incremental usage accounting. It does not delay listener startup
for the whole backfill. Each chunk commits before retention/checkpointing; a crash
resumes remaining NULL rows. Persistent indexing failures have separate health
feedback. Older metadata filters remain visibly incomplete; applying them and
sorting metadata columns wait for indexing. Summary-column sorts remain usable.

History pages contain an Arc-shared Vec of at most 200 rows, so compact and pinned
Kit DataTable delegates do not duplicate their payload. Header sorting runs in
SQLite over all matches, with a typed value/ID keyset cursor and captured maximum
arrival ID for older pages. Latest releases the cursor/ceiling and follows new
reports. Retention may remove old rows while browsing. Sort changes and conditions
reset the cursor. The Kit cycle is default → descending → ascending → default.
Table selection still opens the existing on-demand details/rule workflow.
`history/settings.rs` stores compact/table mode and selected column order in a
4-KiB version-1 `<database>.history.json` document. Saves validate unique known
columns and atomically publish from a private file without holding the snapshot
lock during I/O. Resized widths and header/filter state are not persisted yet;
column order can be changed by removing/re-adding columns, not by dragging.

`app/history_tools.rs` keeps staged conditions/columns and captured search/action/
sort/page scope in one native window. All database/settings/encoding/file I/O runs
outside UI callbacks. Main-view updates use a four-message bounded channel.
Clipboard queries one matching page, encodes it in the background and caps it at
4 MiB. CSV uses quoted RFC-style fields/CRLF and preserves embedded quotes/newlines.
JSON Lines writes selected displayed values as strings under stable column keys.

File export makes a private SQLite backup in 128-page steps, allowing at most
15 seconds to obtain a consistent snapshot under concurrent writes. Source files
are capped at 4 GiB; source/destination readers use 512-KiB suggested caches and
mmap=0. Any remaining legacy metadata is backfilled on that private copy; the copy
has an 8-GiB page cap. Streamed sorting uses SQLite temp files, and output is capped
at 256 MiB with one row and a 16-KiB buffer in memory. Encoding or a slow destination
holds no live history transaction/checkpoint open. Temporary snapshot/sort space
is additional to the history retention target and is removed on normal completion/
failure. Stop is checked between backup steps, private metadata chunks and rows.
Output is fsynced to a private same-directory file and atomically published only
after success. Existing targets require explicit replacement; the live DB, WAL,
SHM and UI preference paths are reserved. Root/directory targets fail without panic.

The focused history-tools GPU flow proves typed UID/command AND filters, invalid
port preservation, Reject quick filter, saved/reopened columns, both header sort
directions, sorted next-page navigation, actual CSV/JSON/clipboard contents and
existing-file preservation. Core tests additionally migrate schema 3 with bounded
legacy detail backfill/unknowns, check literal SQL-looking searches, stable ceiling
and tie-break paging after new arrivals, concurrent snapshot export, cancellation,
reserved-file protection and settings reopening. Native `--feature history-tools`
uses Ctrl+F → Tab/Space to open tools, Enter on Select to choose UID/Equals, types
9999, adds/applies it and requires the main history's empty-state OCR. Native file
pickers, export/column buttons, horizontal scrolling/resizing and interrupted
mid-backfill restart remain separate proof gaps. Native control keys must allow
rendered frames between focus/selection changes; Enter activates the pinned Select.


## Daemon statistics cache

Pinned 1.7.2 `daemon/statistics/stats.go` reports cumulative `by_executable`,
`by_host`, `by_address`, `by_port`, `by_uid` and `by_proto` maps, separately from
its emptied event batch. It can evict low-hit keys; accepted also includes ignored
traffic and DNS responses. Repeated reports replace absolute totals rather than
incrementing them, and accepted/dropped are not an event-derived ratio. Reference:
https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/statistics/stats.go.
Installed UI `service.py` upserts these values into separate statistics tables;
`dialogs/stats.py` supplies category clear, drilldown and export workflows.

Schema 5 adds `statistics_nodes` and `statistics_counts`. Raw u64 values and local
baselines use fixed-width decimal TEXT, avoiding SQLite signed-integer overflow;
aggregation across nodes uses exact u128 in Rust. Local reset records the last
raw count as a baseline. Replayed totals remain zero after reset, increases show
the delta, and a decreased raw count resets that key's baseline. Uptime regression
clears that node's old keys without deleting retained events. Omitted keys keep
their last report. Node statistics expire/prune during subsequent reports, rather
than through a separate periodic worker.

One pending latest sample coalesces under writer pressure and reserves from the
existing 8 MiB ingestion semaphore. Reports are limited to 10,000 keys / 2 MiB
including allocation capacities, with field-specific key bounds. Statistics
skips, errors and eviction counts are separate from event-loss accounting.
Pre-Subscribe reports have one latest slot per connection and remain subject to
the shared ingestion budget; superseded connections cannot submit new samples.
No counter write or reset blocks AskRule or waits for a dashboard window.

The writer caps cached nodes at 32 and counter rows at 10,000 / approximately
2 MiB including conservative key/index overhead. Eviction deletes the minimum
oldest prefix needed to satisfy both caps. Scalar node records are additionally
bounded (up to about 32 KiB). Counter storage reserves space from history's
conservative **quarter** of `--history-mib` logical budget before insertion; this
can reduce retained events. SQLite file/WAL/index overhead still means this is a
target, not an OS quota. A retention deletion publishes a history revision too.

One background read permit captures a consistent read transaction with a 512-KiB
cache and mmap disabled. It aggregates/sorts bounded keys, writes a private binary
view capped at 4 MiB, then releases the live DB transaction. The UI holds offsets,
a top-ten preview and one 200-row page; paging reuses the same file. Live reports
refresh the first page at most once per second. One capture may be pending and
one export window may retain an older snapshot, so temporary disk storage is
additional to retention and bounded by at most three such view files. Kit
BarChart renders the top ten without repeated appearance animation; positions
use f64 while labels/list/export preserve exact integer values.

Export streams captured pages to a private file with a 16-KiB buffer and a 16-MiB
output cap, then atomically publishes after cancellation checks. Clipboard stays
below 4 MiB. It shares history's reserved-target checks for the database,
WAL/SHM and preferences files. Case-sensitive exact history filters support
statistics drilldown; user-facing Equals remains case-insensitive for text.
Selected history deletion uses exact value/node scope and the normal cascading
snapshot/usage triggers. Category resets ignore the text search deliberately;
the confirmation states its whole-category node scope. No daemon commands are
issued by these local operations. Concurrent named Unix sessions are now supported; TCP/TLS and remote peer
identity remain pending (see the independent node sessions section).

`tests/statistics.rs` verifies pre-subscription delivery, replay/omission,
reset/decrease/restart/reopen, all six categories, exact cross-node u64 sums,
frozen all-page snapshots, search/sort/export/cancellation/reserved paths,
case-sensitive history deletion, invalid reports and coalescing under a held
SQLite write lock while a permission is answered. Module tests exercise row,
byte, node, age and allocation-budget limits. `--feature statistics` adds ten
real GPU/input frames; native proof currently covers opening/searching and
persistence only. Native reset/export/chart/picker gestures, cap-sized output,
long-running RSS and real daemon enforcement remain separate proof boundaries.


## Independent node sessions and metadata

`backend::Status.sessions` stores at most 32 current sessions by generation.
`Status.session` is the selected-node snapshot for existing rule views; it is
computed when reading status and is not the routing authority. Backend request
handling, permission decisions, rule editors, historical connection actions and
checksum reviews resolve their own session. Selecting a different node affects
rule browsing, not already opened editors or pending permissions.

A transport owns an atomic current generation and shared admission counter.
Every Subscribe, including a repeat on the same HTTP/2 transport, receives a
fresh generation. The counter is seeded with current nanoseconds to distinguish
persisted metadata across app runs. Node identity currently uses a strict
128-byte non-control name from ClientConfig (empty becomes local); same-name
subscription replaces only that node. Remote peer/address identity remains part
of the TCP/TLS slice. Socket destruction disconnects the transport's latest
session, including clients that never opened Notifications. An old stream's
closure cannot disconnect a new subscription. General transport/task admission
limits remain separate from the 32 registered-node bound.

`rules::Catalogues` replaces global inventory/uncertainty flags with at most 32
per-session gates. A cached inventory or acknowledged change is applied only for
its current node generation; stale replies cannot update another node. Disconnect
and replacement cancel only that node's pending commands/prompts. A timeout or
cache failure invalidates only its gate. Daemon ERROR marks that node uncertain;
raw configuration/interception changes require a fresh inventory before retry.
The daemon may mutate memory before reporting a persistence error, so an ERROR
is not proof of rollback. The sequential rule transfer retry contract is retained.
All nodes share the existing 16-command cap / ten-second acknowledgement deadline;
node configuration/interception commands serialize with all other commands for
that node. Refresh refuses only that node's waiting prompts or commands.

Schema 6 adds `node_info`: generation, kernel version, reported config, reported
log level, interception state and the reported system-firewall protobuf snapshot.
JSON is at most 64 KiB and the firewall snapshot at most 256 KiB per node; versions
are shortened to 256 bytes. Metadata writes share the normal 8-MiB ingestion
semaphore and bounded writer queue. The row limit is 32, preserving active nodes
and evicting the oldest offline cached node, along with its rule catalogue. Normal
history is retained independently. The metadata cap is about 10 MiB across nodes;
rule catalogues remain separately bounded at 10,000 / 2 MiB encoded per node
(up to 64 MiB for 32 cached nodes, with another bounded live-loading set). Existing SQLite max-page targets
may reject oversized caches; UI reports errors instead of inventing successful
storage. Metadata admission also prunes legacy rule catalogues that have neither cached
metadata nor a current live generation, retaining live nodes whose metadata is
still loading. Up to 32 cached plus 32 live-loading rule catalogues can therefore
coexist (a conservative 128-MiB encoded maximum before SQLite page limits).

Statistics now has one latest FIFO slot per node, at most 32 slots sharing the
same 8-MiB semaphore. Replacing a sample keeps its queue position, preventing a
busy node from starving another. Normally one sample is handled between commands;
Barrier and local counter reset drain the captured pending set (at most 32) before
continuing. Queue coalescing/rejection remains explicit statistics health.

The node list reads only summaries, at most 32 rows with a 512-KiB SQLite cache,
and loads one selected configuration on demand. One node read permit keeps
background work bounded. Search clears selection and stale confirmations. Only a
matching persisted generation/live stream enables controls. Interception updates
persist after matching OK; configuration writes use exact reviewed JSON. The
pinned Kit Textarea provides multiline editing with a guarded paste and a
post-change 64-KiB document bound. Configuration review is staged independently
from the stored original, disables editing during confirmation, preserves the
user document on failure and never publishes a success before acknowledgement.
Native close minimizes while a command waits; explicit Close waits for completion.
Typed forms and changed-field review are described below. Broad input-limit stress
and node resource monitors remain pending.

Primary contracts: pinned 1.7.2 `daemon/ui/notifications.go` implements
ENABLE_INTERCEPTION, DISABLE_INTERCEPTION and CHANGE_CONFIG; configuration reload
precedes the final save/ACK. `daemon/ui/config/config.go` defines the JSON object,
and installed UI `nodes.py` / `dialogs/preferences.py` define the workflows.
The test fixture never forwards these commands to a system daemon or firewall.

`tests/nodes.rs` covers two live nodes, crossed ACKs, held rule changes, independent
permissions, same-node replacement, same-transport fresh generations, old-stream
closure, fair per-node statistics/flush, confirmed interception/configuration,
ERROR/persistence/reopen, 32-node admission and offline eviction. The focused GPU
journey drives actual node tab/list, rule targeting, cancel/confirm, held ACK,
another node's prompt, invalid JSON and review/back/apply through Kit controls.
The Nodes native journey covers read/search/persistence with zero commands.
The separate daemon-preferences journey adds native default-action mutation;
native interception, live resource tasks and real-daemon enforcement remain
separate requirements.

## Typed daemon preferences

`nodes/preferences.rs` defines 29 fields and their exact daemon JSON paths, types,
choices and bounds. `app/daemon_preferences.rs` owns one editor with pinned Kit
Select/Input/Checkbox controls and five groups. Form rebuilds own/drop their
subscriptions. Keybindings register once at startup; group scroll IDs retain only
five positions. I/O stays on the existing node/control/history workers.

The dirty-field set patches only intentional changes into the complete reported
object. Unknown and unreported values survive unrelated edits, including unknown
numbers with arbitrary decimal precision. Clearing an optional numeric field
removes its key; clearing a text field preserves an empty string. Missing parents
can be created for additions, while nonobject parents are rejected rather than
silently overwritten. A removal under an absent parent does nothing. Unknown
choice values remain available as reported values rather than acquiring a default.

Fields cover default action/duration, process monitor, unknown-process handling,
rule paths/checksums, log level/file/time formatting, firewall queue/monitor/backend
and config path, startup connection flush and GC target, statistics limits and
workers, server address, auth/certificate policy and paths. Wire auth values come
from installed `auth/__init__.py` (`simple`, `tls-simple`, `tls-mutual` and its
lower-case certificate policies); the Go configuration's comments are insufficient
to infer these spellings. Connection settings configure the daemon client, while
Rustleblower's transport remains Unix-only. Daemon paths are not opened locally.

Text fields have purpose-specific 128/256/1024/4096-byte limits. Queue numbers are
u16, statistics limits are nonnegative i32 and the GC target is a signed integer.
Firewall monitor intervals accept positive Go-style compound/decimal durations.
Exact integer parsing checks the signed nanosecond range, avoiding f64 rounding
at its boundary and rejecting intervals that truncate to zero. The final JSON
object remains capped at 64 KiB. The raw Textarea also guards paste and restores
its last bounded value after an oversized edit.

Settings/Complete JSON switching synchronizes a candidate without issuing a
command. Review compares against the confirmed original recursively, retaining
at most 200 changed rows with shortened paths and before/after values. Array
changes appear as one value. Review gives the full count and points to Complete
JSON when rows are omitted. The candidate and original each remain document-bound.
Back preserves edits; Confirm submits only the staged candidate to its original
session. Ctrl+Enter reviews and Ctrl+Shift+Enter confirms after review. The shared
16-command/ten-second ACK bounds apply. Only matching OK plus persisted cache
updates the confirmed original. ERROR keeps the candidate/review and last-confirmed
cache, disables another apply under uncertainty and permits returning to editing.
Close minimizes during pending ACK; no telemetry or SQLite wait blocks a prompt.

Five model tests cover precise preservation/defaults, removals/scalar parents,
field/type/size/duration validation, wire auth strings and bounded escaped Unicode
review paths. The focused eleven-frame GPU journey edits every group, rejects
negative limits before sending, switches review/back/JSON/settings, checks exact
payloads, holds ACK while answering Allow once, verifies persisted success, and
checks rejected-save preservation. Native verification edits Default action with
the actual Select and keyboard review/confirm, then checks the synthetic ACK and
SQLite data. The synthetic Python client accepts CHANGE_CONFIG only under an
explicit test flag and never forwards it or writes system configuration.

Apply-to-all-nodes, exhaustive native field variants, actual TLS handshakes,
cap-sized interactive stress and real-daemon enforcement remain pending. Evidence
and commands are in the daemon-preferences feature map and aggregate report.

## Linux releases

Only Linux x86-64 is packaged. `rust-toolchain.toml` pins Rust 1.99.0. The Linux
workflow installs Ubuntu 24.04 build/graphics dependencies, prepares the scoped
h2 patch through `scripts/cargo.py`, runs core/helper/Clippy checks and every
background GPU journey. A `v*` tag calls the same checks, builds the release and
runs `scripts/package-linux.sh`. That script creates `.deb`, `.AppImage` and
`.tar.gz` assets plus SHA256SUMS. AppImage tools/runtime have pinned checksums;
graphics drivers and glibc remain host dependencies. Packaging is adapted from
FerryDock under its retained MIT notice; no macOS or Windows jobs are included.

`python3 scripts/verify-packages.py` checks checksums and starts packages in
disposable Ubuntu containers. It verifies version output and the explicit
missing-display error before any socket is created. It does not contact a host
daemon. These are installation/library/startup checks, separate from GPU/input
proof. A release uses a matching Cargo version, annotated tag and
`docs/releases/v<version>.md`, and publishes only after all checks/packages pass.
The first release is explicitly marked as a development prerelease.

The initial v0.1.0 tag failed its GPU gate and has no published release. Ubuntu's
DejaVu font exposed a countdown-status wrap that moved a checksum button between
pointer down/up. The status now stays on one line; the checksum journey pins
that font and retains its first-click state. v0.1.1 is the first publication
candidate after that correction; the original tag remains immutable. Checksum
review also resolves the requesting session independently of the browsed node.

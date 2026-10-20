# Blocklist subscriptions and rule collections

Reference: OpenSnitch a1353848, source version 1.9.0. Read
[upstream reference](../../../../docs/upstream-reference.md) and
[collection contract](../../../../docs/subscription-collections-plan.md).
This is the GUI-side subscription runtime, separate from daemon downloader tasks.
The full subscription port remains incomplete.

## Native controls

Blocklists is a main tab (Ctrl+7). Empty setup is usable immediately; Add or Ctrl+N
creates config through normal fields, and Ctrl+S validates/saves. No manual JSON
bootstrap is needed. Inputs cover name, URL, filename, format, enabled state,
groups and inherited interval/timeout/size. Defaults covers global limits and UA.
Controls include conditional/forced refresh, automatic start/pause, current/queued
cancel, error/backoff/status, bounded preview/logs/clipboard, global matching-set
sort/search/paging, removal with retained data and restoration.

**Rule collections…** / Ctrl+Shift+L opens all-list and normalized group targets,
separated by reader kind. An empty installation has five zero-source all-list
collections. Search, Group/Type/Active sources sort, descending direction and
64-row paging apply to the entire matching catalogue. Groups become available
when enabled subscriptions have compatible retained data. Runtime Pause stops
refreshing while existing data remains active; source enabled membership is a
separate setting.

Enabled/group/input-type edits and restoration of existing subscriptions require
a before/after membership review. Back preserves staged fields; Cancel preserves
saved metadata; confirmation uses the original revision. Review also applies if
the original form snapshot had no file, covering downloads completing during
editing. Removal confirmation explains collection removal and retained direct-
list files. Existing daemon rule definitions are not rewritten implicitly.

**Review new rule…** resolves the exact current single or collection target on
the worker, captures a local node session and opens the ordinary rule editor.
The initial draft is enabled Deny/Always with exact local UID AND watched-directory
predicates. The UID comes from the kernel, not an environment variable. The
editor shows the target node, supports its normal metadata/conditions/action/
duration controls, rejects existing names and waits for ACK plus durable cache.
Cancel sends no command. Window-manager close currently minimizes this editor;
use its Cancel action to discard. This legacy close convention remains part of
the required final UI/UX assessment.

**Associated rules…** scans a read-only confirmed inventory snapshot over every
catalogue page and excludes unobserved popup proposals. It matches exact operand
and directory in direct or nested operators, retaining 16 rules / 512 KiB. Its
pagination keeps the captured node and target, independently of another selected
subscription/node. Opening an entry rereads and compares the confirmed rule;
changed/deleted snapshots are refused until Refresh snapshot. Unrelated fields
remain intact. A bounded completion receiver clears busy state after an
interrupted window-bound lookup; explicit in-flight close/reopen proof remains
pending beyond normal-completion tests.

## Reviewed bulk configuration

Bulk edit / Ctrl+Shift+B uses marks retained across catalogue pages or every active
match of the captured search. Filter changes and Show removed clear the marks;
sort/page changes preserve them. Mark page and Clear marks are explicit controls.
The form covers the pinned upstream bulk controller's enabled, groups, input
format, interval, timeout and maximum-size fields; Add/Remove/Clear group modes
extend its replacement action. Checked limits now retain optional count and unit
independently through the existing Kit selectors; unchecked limits keep each
source's existing override. Empty count and Inherit default unit can be combined
or chosen separately. The pinned bulk view disables units when count is zero
and serializes both as null in that case; native unit-only bulk editing is a
consistency extension following the shared pinned models and individual editor. Group
changes use normalized collection keys, preserve order and remove duplicates.
The entire proposed change is shown before applying it.

The worker freezes at most 128 targets into before/after entries (2 MiB cap).
Unchanged entries count in the scope but are omitted from the write plan. Review
shows 16 changed entries per page. Back retains fields; Cancel performs no write;
Rebuild resolves current selected IDs or current matches of the captured filter.
The review captures one bounded defaults snapshot and revision, shows its count/
units and effective before/after seconds/bytes, and validates each dependent limit.
Applying revalidates these defaults and every changed row's revision and exact
configuration inside one SQLite transaction before the first update. Any stale,
removed or missing changed row rejects the entire batch. SQL errors roll back all
metadata and log writes. Rows newly matching after review stay outside that plan.
A successful batch advances only changed revisions, retains names/URLs/filenames,
resets HTTP validators only for changed input formats, cancels affected active/
queued downloads and synchronizes collections once. Failed validation/SQL does
not cancel downloads. Existing policy definitions are never rewritten implicitly.
The shared collection filesystem/DB crash-recovery limitation still applies.

Ctrl+Enter reviews and Ctrl+Shift+Enter applies. The status picker avoids matching
both Keep and Enable when searched. Opening the review explicitly restores the
retained dashboard focus, so keyboard confirmation survives removal of the
previous form controls. Native input proof and exact backend proof are distinct.

## Configuration transfer and migration

Import / export… / Ctrl+Shift+T accepts Rustleblower versions 1/2, the pinned
OpenSnitch list_subscriptions action envelope, or its config object. Typed file
paths use Kit Input; format selection, replacement/default choices and actions
use Kit controls. Ctrl+Shift+I is reserved by the pinned GPUI inspector and must
not be reused. Ctrl+Enter reviews; Ctrl+Shift+Enter applies. Back retains staged
file/options, Cancel returns to the actual catalogue without writes, and Rebuild
rereads the file. Apply uses the exact captured document even if its source file
changed after review. The source is never normalized or autosaved in place.

The worker reads a regular nonsymlink JSON file using O_NOFOLLOW/O_NONBLOCK,
rejects depth >16, >16384 nodes, duplicate keys, objects >128 keys, keys >256
bytes, arrays >256 and strings >8 KiB before Value allocation, then validates at most 128 subscriptions
and a 2-MiB complete document. Before queueing, a complete import review is
capped at 6 MiB across its document, metadata, before/after entries and removals. Paths are absolute and at most 4096 bytes. Native
version-2 export includes all saved settings; OpenSnitch export preserves
independent count/unit choices and its envelope. Both exports use a consistent SQLite read transaction, private
temporary file, fsync and no-clobber publication. Marked rows or every captured
active match can be exported. Selected exports rebuild embedded templates from
only their selected subscriptions, excluding unselected source records.

OpenSnitch conversion covers global/per-list units, compact durations, filename
derivation/type suffixes/collisions, legacy group plus groups grammar, reserved
all and duplicate labels. Empty count and unit overrides inherit separately.
Independent overrides remain dynamic after later defaults changes; old numeric
native limits keep their absolute seconds/bytes meaning. Source unit aliases are
canonicalized, compact durations become absolute seconds, and unit-only overrides
retain null counts on export. Earlier imports that already discarded raw choices
need explicit reimport/edit; do not guess their original intent from a number. Unknown envelope/action/config/per-source fields and notify data
are retained in a bounded versioned SQLite template for export, and never run.
Source lists_dir and cached publications are not adopted. Unknown root metadata
from the newest source supersedes prior root metadata. Per-source extra fields
are stored separately by immutable internal ID, so names may repeat and ordinary
renames/restores do not detach or exchange those fields. Native metadata arrays
are ordered with their exact name/URL/filename/reader identities; mismatched
embedded arrays are refused. Native export preserves duplicate names/filenames;
OpenSnitch export normalizes typed filenames and makes filename collisions distinct
without changing display names. This does not implement other plugin execution
or subscription notification delivery.

Review has 16 entries/page and includes exact before/after configuration plus a
compact comparison when global defaults (including automatic updates) are chosen.
Defaults are kept unless explicitly selected. Merge replacements/restores require
their checkbox. Matching prefers URL/reader/typed filename, then unique URL/reader
or name; exact saved configuration, retained extra fields and a matching name
resolve duplicate candidates. Indistinguishable candidates use the first unused
ID in stable order, shown in review. Each existing ID is used at most once.
Unresolved merge ambiguity asks for complete catalogue replacement rather than
renaming source data or guessing an attachment.

Complete replacement is a separate checkbox. It includes both imported entries
and every absent active ID in the 16-row review pages (at most 256 rows total).
Matching entries may be replaced; unresolved ambiguous matches become new
identities, and old unused identities are explicitly retained removals. Removal
stops updates and group/all membership, preserving files and daemon rule
definitions. Back/Cancel do not change the catalogue. Capacity is checked before
writes (128 active/256 retained), counting retained removed records. Applying
revalidates catalogue IDs/revisions, settings and metadata revision in one
transaction. Stale
state or a late SQL failure rolls back insertions, updates, logs, defaults and
metadata, archives and removals together. The archive count is read before commit
and published afterwards, with no SQLite work under the UI-state watch lock.
A successful commit cancels affected downloads and synchronizes
collections once; filesystem/SQLite crash recovery remains a separate limitation.

## Independent count/unit inheritance

Pinned `models/subscriptions.py::SubscriptionSpec.from_dict` resolves missing
counts and units separately against `GlobalDefaults`. Add/edit/default forms now
use three Kit count inputs and unit selectors. Either part can inherit; effective
seconds/bytes are displayed with the loaded defaults. Existing numeric native
values remain absolute; new structured overrides retain optional count/unit.
Native export is version 2 and still reads version 1. Global normalized values
also retain units so the raw default count can be recovered exactly. Previously
normalized old imports cannot be unambiguously reconstructed: reimport the
original file or explicitly edit those limits.

The existing resource bounds remain: interval 60 seconds–31 days, timeout 1–300
seconds, a maximum list of 1 KiB–100 MiB and the retained 20–2048 MiB budget.
Unsigned conversion/checked multiplication and unit domains reject overflow or
invalid combinations. A default save validates every retained spec (at most 256)
inside the SQLite transaction, streaming one 8-KiB spec at a time. Successful
normal schedules recompute from last check plus new effective interval; existing
failure backoff is preserved. Active download cancellation keeps its existing
cooldown behavior. Default/log/schedule failures roll back, and an individual
save commits before affected downloads are cancelled. The editor's captured
default revision is checked before its save; a stale form must be reopened.

Import review resolves overrides against the selected imported or retained
defaults and shows default count/units plus effective limits. Incompatible
dependent limits reject the whole review/save. Canonical OpenSnitch export emits
null counts/units where inherited instead of freezing their current values.
The source's permissive parser/serialization quirks are not copied: invalid
resource values are refused and compact durations export as explicit seconds.

`--feature subscription-inheritance` adds twelve Kit checkpoints for individual/default
count/unit selection, save/reopen, changed default counts and units, count-only
overrides, invalid dependent refusal, permission input, stale-default guard,
fresh reopen and legacy absolute preservation. Four core cases add old-v1/v2
roundtrips, atomic late SQL/log rollback, checked schedule recomputation,
incompatible retained defaults and an actual HTTP download that fails at 1 KiB
then succeeds when an inherited count raises its cap to 32 KiB. Full core is 174.

Run `verify-subscription-interchange.py` with `--inheritance-cases` pointing to the
fresh `RUSTLEBLOWER_INHERITANCE_EVIDENCE/rust-inheritance-cases.json` produced by
the core case. Alongside the normal twelve interchange checks, it compares five
actual Rust configurations across three default phases (fifteen effective-limit
tuples) with the unmodified pinned pure Python models. No Qt/plugin runtime runs.
Native `--feature subscription-inheritance` proves real keyboard unit-only input
and exact persisted null-count/minutes/seconds/KB values, three PNGs, no download
or policy command, 430 events and complete cleanup. Recheck native subscription
import and URL validation: unit selectors add three Tab stops before Test URL.

Bulk actions now preserve untouched structured limits and offer the same
independent count/unit choices as individual forms. The source bulk view's
zero-count-clears-both behavior is retained through empty count plus Inherit
default; unit-only selection is a native consistency extension. Native default changes,
stale-editor/reopen controls, native bulk-unit gestures, real scheduling timing,
package/RSS and final UX
remain separate.

## Legacy metadata migration

Schema 2 stores only extra fields in a per-subscription table and global action
metadata in the singleton. The combined global/bound/archive JSON is capped at
2 MiB; bound and archive tables each admit at most 256 records. Budget checks
precede JSON allocation on reads. A one-time transaction migrates the earlier
name-indexed template: whole-template normalization first preserves filename
collision order; uniquely matching source identities or names bind to IDs.
Uncertain/orphan records remain in a separate archive without data loss. The
Blocklists page shows their count. Reimport of matching original source records
binds fields and removes exactly those archive records in the same transaction.
The migration marker prevents rebinding on later starts. Archive inspection and
manual ambiguity resolution/cleanup are Rustleblower recovery improvements,
not an upstream UI page or an additional source-backed parity requirement.
Keep the records and the existing reimport recovery until a separate recovery
interface is implemented; do not discard them to simplify migration.

## Test a URL before saving

The pinned `ui/workers/url_test_worker.py` and subscription dialog expose an
explicit URL test. Add/edit now offers **Test URL** and **Cancel URL test** in
the fixed footer. It works before a name/filename is valid, never saves metadata,
publishes a list or edits a daemon rule. Saving is a separate action and does not
require a successful test.

HEAD checks reachability with a three-second deadline. Only 403/405 falls back to
GET; hosts format uses a five-second GET sample (one GET suffices for both fallback
and sampling). Other formats check reachability without a body sample. Redirects
are capped at five and each target must remain a bounded HTTP(S) URL without
credentials/fragments; TLS verification stays enabled. Each admitted diagnostic task
has a twelve-second ceiling and uses the saved HTTP user agent. Hosts samples stop
at 200 lines or 64 KiB, preserve split stream lines and ignore invalid UTF-8 as
the pinned helper does. This is not the full downloaded-list normalizer.

The helper accepts at most ten non-comment sample lines without a conclusive
format check, otherwise requires at least 60% hosts-like lines. The UI labels a
short sample as inconclusive; a bad sample leaves the form editable. HTTP failures,
redirects, fallback and capped samples have explicit feedback.

There is one probe task separate from the existing download, a bounded 32-message
worker queue and one owned reply. A second probe is refused. Ticket drop cancels
the request. URL/format edits discard prior results, Cancel/Save/form destruction,
leaving Blocklists and closing the dashboard cancel only the probe. Page/settings
requests, normal downloads and permissions remain independent. No HTTP work occurs
on the UI thread; cancellation does not wait for a worker command queue.

Run `python scripts/verify-headless.py --feature subscription-url` in its owned
loopback namespace. Require eleven JSON checkpoints: empty URL refusal without HTTP; redirect
and hosts sample; invalid hosts; changed-format HEAD-only; held GET with actual
AskRule/Deny once; cancel; edit discards stale result; form cancel/reopen; leave/
return cancellation; and an explicit Save as the first subscription/log mutation.
The full subscriptions journey remains a separate regression.

Six `tests/subscription_probe.rs` cases cover the URL/heuristic bounds, HEAD/GET
403/405/error/redirect/loop/oversized-target handling, split/chunked/long/many-line
samples, non-host HEAD-only, broken bodies, TLS protocol failure, HEAD timeout,
busy/cancel/drop/shutdown, worker queries and real permission independence. A held
ordinary download stays active through probe success/cancel and retains its
previous published bytes. Full core now contains 170 cases.

For independent source comparison, set `RUSTLEBLOWER_URL_TEST_EVIDENCE` to a fresh
directory when running the focused core test, then run
`python scripts/verify-url-test-heuristic.py --fixtures /absolute/path/rust-hosts-samples.json --evidence /absolute/new/proof`.
It executes only the unmodified pinned pure `is_hosts_file_like` helper against
nine actual Rust results; no Qt/plugin runtime is started. Native input, actual
external endpoints, certificate trust failures, compression and package/RSS
measurements remain separate from those core/Kit proofs.

After a desktop rebuild, `python scripts/verify-native.py run --feature subscription-url`
adds actual KDE keyboard access to the fixed Test URL footer and unsupported-
scheme refusal before a request can start. Require three PNGs, visible refusal
OCR, zero subscription/log rows, zero policy commands, 430 history rows and all
cleanup flags. This uses no external HTTP endpoint and proves native validation
input, not native successful HTTP/cancellation or a panel pointer click.

## Storage, formats and resource bounds

Downloads buffer normalized output in 64 KiB and yield between at most 64 KiB
of parsing, including when HTTP chunks are already buffered. The large-chunk
cancellation test runs on a single-thread runtime and requires cancellation
before the whole ready chunk is consumed. The 20 MiB publication-budget test
retains its existing request/status deadlines and last-good-file assertions.

`backend::subscription_tests::permissions_reply_while_a_real_bulk_sql_transaction_is_busy`
holds the backend's actual subscription connection inside an SQLite UPDATE
hook. It waits for explicit entry, independently proves the real write lock,
requires Ping and Allow within the existing 500 ms deadlines while the bulk
reply remains pending, then releases the hook and checks both committed rows.
The hook/setup entry point exists only in library test builds. This replaces
the CPU-speed-dependent 20-million-step recursive SQL trigger and its startup
sleep. Run the normal no-default-features core suite to include this test.

One separate worker/current-thread Tokio runtime owns a separate WAL SQLite DB,
manager directory lock and all HTTP/filesystem I/O. Its errors do not stop the
server or permissions. A second manager refuses the same directory. The command
queue holds 32 requests; one download runs at a time, with at most 128 queued IDs.
The catalogue is capped at 128 active / 256 retained entries, 64-row pages, 1024
logs / 100 displayed and a 64-KiB preview. Defaults are paused, 24-hour interval,
60-second timeout, 20-MiB file cap and 512-MiB published-data budget. Intervals are
60 seconds–31 days, timeouts 1–300 seconds and files 1 KiB–100 MiB. Settings bound
the disk budget to 20–2048 MiB and at least one maximum file.

The HTTP client initializes lazily, supports HTTP(S), trusted native TLS roots,
five redirects and gzip. Actual HTTP verification uses only owned plain loopback
servers. Input and normalized output have independent configured byte caps;
lines are at most 8 KiB / two million total. The normalizer expands hosts aliases,
omits localhost/nonblocking hosts mappings, writes domain/pattern names as hosts
rows, retains mixed IP/CIDR, reduces MD5 to lowercase hashes, and preserves regex
whitespace/literal `#`. Go retains responsibility for regexp compilation. ASCII/
IDNA domain names are required; unsupported brace alternatives are rejected.

Files live under `sources/<id>/<reader-kind>/download-<filename>`, with an extension
for the daemon's `*.*` glob. Hosts and domain names share the domain reader. The
publication registry records source URL/format/path/bytes; legacy unnormalized
rows and changed URL/format cannot create direct rules until refreshed. Old typed
publications are retained for existing direct rules and included in disk usage.
Failed, invalid, oversized or canceled HTTP retains last-good bytes; revision and
cancellation checks discard obsolete in-flight results. A publish fsyncs/renames
data before saving metadata; filesystem/SQLite crash atomicity is not claimed.

Collections use stable `collections/current/all/<kind>` and
`collections/current/groups/<slug>/<kind>` paths. Owned generations contain source
symlinks plus a comment-only refresh marker, with at most 1152 source links total
(128 × [one all + eight groups]). Only enabled/nonremoved compatible publications
are active. Changed URLs keep last-good compatible data. A stable marker and
persisted publication epoch force daemon reloads even at equal file counts or
equal source sizes/mtimes. Unchanged membership/data signatures do not rebuild.
Only marked owned generations can be cleaned; unknown pointers, directory
symlinks and unmarked debris are refused. See the collection contract for recovery
limits and the distinction from retained direct-list activation.

## Verification

- Full core: `python scripts/owned_net.py --evidence /absolute/new/network.json -- python scripts/cargo.py test --no-default-features --locked`.
  Coverage includes 70-row catalogue queries, all six canonical inputs, stale
  revisions, locking/reopen, HTTP validators/304/force, stream/header limits,
  errors/timeouts/cancel/edit/remove/shutdown, retained data/budget, independent
  Ping/AskRule, 221-rule attachment scans, 85-collection global query/sorts/direct
  lookup, typed active membership, generation guards/recovery and coarse-mtime
  reload epochs. Bulk cases cover 70 targets across pages, nonmatching/new matching
  records, stale and tampered reviews, a failure on the last SQL update with no
  preceding writes/logs retained, reopen, group limits/modes, inheritance/overrides,
  retained canonical files and explicit format validator invalidation. A real
  constant-memory SQLite trigger delays only the owned subscription transaction;
  the test requires its actual write lock before and after production Ping and
  AskRule/Allow replies, with 500-ms RPC/decision bounds. A separate kernel-UID probe checks UID 1000 outside the namespace.
- Kit: `python scripts/verify-headless.py --feature subscriptions` has **61 frames**.
  It uses actual controls for the complete catalogue/download journey, UID-scoped
  direct and group rule review/cancel/ACK, an independent permission under held
  ACK, associated-rule paging and rejection, concurrent confirmed edit/stale
  snapshot refusal/refresh, membership review/back/cancel/confirm, five empty
  all-list kinds, 86 collections across pages, every sort direction and global
  filter. Bulk input adds cross-page marks, Back/Cancel, concurrent-edit refusal
  and rebuild, group-cap rejection, global disable, selected re-enable and checked
  inherited/custom limits; it independently inspects durable values and requires
  zero implicit policy commands. Transfer input adds invalid JSON, conflict/replacement,
  review paging, Back/Cancel, defaults, concurrent catalogue refusal/rebuild,
  retained data, selected native/all-filter OpenSnitch exports, unknown metadata,
  no-clobber behavior and actual catalogue restoration after cancellation. The
  compact global comparison keeps review controls reachable. Identity input adds
  actual source rename, duplicate display names, per-ID native exports, 84 absent
  active sources in full replacement, removal review paging, Cancel/apply and
  restoration retaining both bytes and extra fields. No download or implicit
  daemon policy command occurs. Data seeding uses production backend APIs; product actions use input.
  Require network ownership, `real_gpu: false`, every JSON checkpoint and owned socket/process cleanup.
- Transfer reference: `scripts/verify-subscription-interchange.py` loads only
  exact-pin pure Python models/helpers with owned XDG stubs. Nine actual Rust
  decoder cases plus the worker's real action export match effective values.
  Two more checks prove duplicate names produce a saveable OpenSnitch action with
  unique filenames, while native backup preserves filenames and ordered fields
  (12 checks total).
  Source and fixture hashes are retained, and fixture writers refuse existing
  files. No Qt, plugin, downloader, daemon or network execution is linked.
- Native transfer: `python scripts/verify-native.py run --feature subscription-transfer`
  has three owned Wayland captures for path input, exact review and keyboard
  import. Independent SQLite reads require legacy native-test/120-second/2-KiB
  conversion, disabled state, unchanged defaults, checked=0, bytes=0, no file,
  zero policy commands, 430 history rows and all cleanup flags. It proves this
  native import path; native export/pickers, all fields and paging are separate.
- Native catalogue: `python scripts/verify-native.py run --feature subscription-catalogue`
  has three owned Wayland captures. It creates an old row, reviews two same-name
  imports plus its retained removal, and applies by keyboard. Read-only SQLite
  requires exactly two active/one removed records, extra fields on IDs 2/3, exact
  source URLs, unchanged defaults, checked=0, no files/bytes or policy commands,
  430 history rows and every cleanup flag. This is native replacement proof;
  native retained-file restore and archive workflows remain separate.
- Native: after a desktop rebuild, `python scripts/verify-native.py run --feature subscription-collections`
  has **four Wayland captures**: usable empty collection page, current UID 1000 in
  review, the exact directory tail through Tab/End, then empty search after
  keyboard Cancel and restored Ctrl+F routing. It retains zero subscriptions,
  automatic updates paused, zero policy commands, 430 telemetry rows and five
  passing cleanup flags. It makes no HTTP request. Name OCR normalizes typographic
  dashes; exact wire predicates are checked independently in Kit/core proof.
  This native flow does not prove saving, membership changes or paging.
- Native bulk: `python scripts/verify-native.py run --feature subscription-bulk`
  adds eight Wayland captures: UI-created metadata, actual searchable status Select,
  exact true→false review and keyboard application. Read-only SQLite inspection
  requires the single expected configuration (only enabled changed), checked=0,
  bytes=0, no file, paused automatic updates, zero policy commands, 430 history
  rows and every cleanup flag. This proves that native mutation path; group/limit
  controls, review pagination and concurrent-edit refusal have separate Kit/core
  proof. Allow rendered frames after opening/filtering the Select before Enter.
- The separate `--feature subscriptions` native journey remains metadata setup
  proof. Rebuild desktop after no-default-features tests overwrite its binary.
- Source-backed Go: `python scripts/verify-list-matching.py --evidence /absolute/new/path`
  uses the retained read-only exact-pin repository (`--git-dir` can select another
  local copy) and installed Go. Actual Rust-normalized files feed unmodified
  pinned reader/matcher bodies. Seven tests cover exact/suffix/label-glob cases,
  hosts aliases, mixed IPv4/IPv6 CIDR, MD5, Go regex, atomic file replacement and
  equal-count replacement via marker. Logging is inert and no interception/RPC/
  daemon-main packages are linked. Source, payload hashes and logs remain only
  in ignored evidence; no daemon/kernel enforcement is claimed.

The HTTP wrapper requires util-linux, iproute2 and unprivileged user/network
namespaces. It refuses parent networking, unmapped/root host UID and extra
interfaces before configuring only its own loopback. There is no host-firewall
fallback. Preserve previous failed verifier/native runs and their scoped cleanup.

## Full parity still open

inactive/old-format inspection and cleanup;
direct-list activation migration; complete filesystem/SQLite crash recovery;
quiet schedule/backoff timing; HTTPS/TLS failure/redirect/compression fixtures;
broader pattern, native mutation/paging, package and memory/RSS proof. The full
goal also retains port ranges, every other pending UI feature, final coherent
UI/UX and the requested upstream issues/PR/Little Snitch research. Build/input/
protocol/cache proofs do not establish real daemon kernel enforcement.

The metadata archive recovery interface is a separate Rustleblower improvement,
not a prerequisite invented for OpenSnitch feature parity. Its retained records
and existing reimport behavior must remain preserved throughout the port.

Subscription result notices are now implemented through the existing bounded
mailbox, with a source-compatible Notifications editor, stale metadata revision
guard and explicit Blocklists action target. Worker, private D-Bus and Kit input checks cover these notices;
see [notification contract](desktop-notifications.md) and
[implementation details](../../../../docs/subscription-notifications-plan.md).

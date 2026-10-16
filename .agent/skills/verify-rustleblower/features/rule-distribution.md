# Reviewed rules across nodes

Reference: OpenSnitch a1353848 (source 1.9.0),
`dialogs/ruleseditor/nodes.py::load_all` and `dialog.py::add_rule/delete_rule`.
Upstream sends the same ChangeRule to a selected node or all connected nodes.
Existing confirmed copies, direct unsaved drafts/edit/rename and bounded
file/directory/clipboard imports share one reviewed destination engine.
Reviewed multi-node folder export is also implemented below.

## User controls

Application rules → **Copy to nodes…** opens a native Kit window. Marked names
across pages take precedence; otherwise the highlighted rule is used. With no
selection, the captured matching filter is the source. The scope checkbox can
expand selected names to every page of that filter. Source node/filter remain
captured while the main page changes. No source policy is modified.

Check individual destinations or Select all destinations. Catalogue copies exclude
the source; drafts and imports include the currently selected node by default;
at most 32 named connected Unix sessions can be captured. Refresh destinations
explicitly reloads current connection generations while retaining staged choices
by name. Review copy freezes source values plus destination identities, counts,
configuration/inventory fingerprints and replacement choice in one short SQLite
snapshot. No command is sent before Apply. Back preserves setup choices; Close
abandons review. Window-manager close currently minimizes this existing generic
editor style; the required final UI/UX pass still covers that convention.

Existing names skip by default. Replacement is an explicit setup checkbox.
Identical complete wire values and names awaiting observation always skip.
Review shows per-destination planned/skipped/identical/awaiting counts and blocks
unsupported port ranges or projected catalogue overflow before changes. The
Source rules view is virtualized and can copy each complete JSON document.
Paths, UIDs, networks, timestamps, flags, action, duration and all wire operators
retain exact source values; the user chooses suitable destinations. This slice
has count-based conflict review and exact source JSON, not a per-target operand
diff inspector.

Apply/continue processes destinations and rules sequentially, one ACK at a time.
Results show counts and a bounded error for each node. Copy results preserves the
full result text when a table line is truncated. Stop waits for the current reply
and keeps the next cursor. Resume excludes confirmed work. A node error leaves
confirmed copies in place and permits other destinations to continue. ERROR or
unknown state requires reconnect and rebuilt review; the frozen old session is
never silently retargeted. Rebuilding recognizes exact already-copied values and
skips them. There is no cross-node rollback.

## Concurrency and bounds

A command reservation owns only the current destination. It refuses overlapping
rule/node/firewall mutations and explicit refresh from other app views. Captured
generation checks still stop an external reconnect. Permission replies and
telemetry use independent paths. After reserving, a bounded history-queue fence
waits for previously acknowledged cache writes without forcing a checkpoint.
The engine releases all SQLite statements/connections before awaiting any RPC.

At review: at most 10,000 source rules / 2 MiB wire / 8 MiB owned rule allocations,
plus 32 compact target summaries and 32-byte source hashes. Source inventory
sizes are checked before loading rule blobs. One destination inventory at a time
retains names (at most 2 MiB), 32-byte hashes, size and observation flags; full
rule bodies are not multiplied by the number of targets. No review matrix or
unbounded jobs list is created. The Cartesian work bound is 320,000 rule/target
pairs. Existing 16 pending commands / 8 outbound / 32 SQLite-command bounds apply.

Before each mutation, the indexed current name/hash/observation state and node
configuration/generation must still match the reviewed inventory. Source and
target byte/count projections are bounded; oversized or duplicate stored rows
are refused before blob allocation. A transient expected-hash map incorporates
our own confirmed writes; Stop/failure retains its digest and next cursor for
safe retry, without retaining a SQLite read snapshot or all destination maps.
Progress uses one latest-value watch slot, at most 32 target records, and errors
at most 1024 bytes. At most one source-JSON serialization is in flight; that
clipboard payload is at most 512 KiB. Copy results is bounded to target summaries.

The background engine survives while its window exists. Dropping the view sets
its atomic Stop flag: an already-sent reply can finish, but unsent targets stop.
No filesystem or network work runs on the UI thread. Source snapshots remain
immutable even if a later source report changes.

## Proof

Run core through the owned-network wrapper as required by the main skill. Full
core has **146 cases**. `tests/cross_node.rs` has six real gRPC/SQLite cases.
The original three cover:

- 230-rule source, 201-name selected scope reaching a later page, default name
  conflicts, identical values, awaiting observations, exact wire preservation,
  held ACK/no optimistic cache, independent AskRule, successful actual WAL
  checkpoint while ACK is held, command/refresh reservation, Stop/retry with no
  repeated successful command and full source preservation. The writer also
  checkpoints; the assertion permits transient competing-checkpoint contention
  but requires actual success within two seconds while the ACK remains held.
- One target rejects while another completes; retry on uncertain state sends
  nothing, then a fresh generation/review copies only unfinished target values.
- Concurrent confirmed destination edits, captured-session replacement,
  unsupported ranges, projected 10,000-rule overflow and oversized source blobs
  refuse unreviewed writes.

`python scripts/verify-headless.py --feature cross-node` records **14 frames**
with actual Kit input: mark 201 names across pages; choose all destinations;
review selected and full-filter scopes; copy exact JSON; hold ACK and answer an
independent permission; Stop/resume; inspect/copy partial failure results; refuse
uncertain retry; refresh staged destination generations after fixture reconnect;
recognize completed copies; explicitly replace a conflict; force-destroy the
owned window under a final held ACK and require no unsent target mutation.
Inventories/reconnections enter through the production daemon fixture boundary.
The general 48-frame GPU regression covers existing rules/editor/transfer/bulk
and permission behavior after the shared snapshot-helper/command changes.

These prove owned protocol/cache/GPU input, not native compositor controls,
network-wide/kernel enforcement, comparative RSS or completed whole-app UX.
Native OS picker interaction remains unproved for imports. Retain all
`.verification/` reports, failed attempts and screenshots; clean only owned
processes and sockets. Rebuild the desktop after no-default-features core tests.

## Unsaved drafts and ordered rename

The shared rule editor's **Apply to nodes…** validates and captures the current
fields before opening the destination window. The original node is selectable;
Cancel/Close restores the staged parent editor without sending a command. The
parent stays busy while this child exists. Same-name edits default to replacing
conflicts; create/rename require explicit replacement of a conflicting new name.

Review shows the exact new JSON and each destination's original-removal count.
For a renamed existing rule, each node confirms the new name and its durable
cache write first, then receives name-only DeleteRule for the old name. This
uses the pinned daemon's `handleActionDeleteRule`, which reads the rule name.
An absent original means create/update only; an awaiting original blocks that
node. Capacity is checked for both names before removal.

Stop after new-name ACK retains the original and its removal cursor. Continue
then sends only the outstanding deletion. Removal failure explicitly retains the
confirmed replacement count and failed stage; fresh inventory/review can skip
the identical new name and remove only the old. There is no distributed rename
transaction or automatic cross-node rollback.

## Immutable imports

**Application rules → Import to nodes…** offers a typed file/directory path,
Kit picker and **Capture clipboard**. Existing bounded transfer parsing accepts
one object, arrays and adjacent objects. Raw clipboard text is limited to 8 MiB;
the shared admitted source retains the 10,000-rule/2-MiB-wire/8-MiB-allocation
limits above. Path input is at most 4096 bytes. Duplicate names are refused.
File I/O and JSON parsing run on the background executor.

Review freezes the parsed source once; later file or clipboard changes cannot
alter Apply. Malformed or oversized captured clipboard data does not silently
use the file instead. **Use file path** explicitly clears the captured source.
Default skips/replacement, session/configuration guards, per-node ACK/results,
Stop/continue and independent permission handling use the same engine.

## Additional proof

The other three core cases cover shared-Arc draft admission, ordered new-name
ACK then old-name deletion on two nodes, Stop/resume between those stages, a
late SQLite DELETE failure preserving both cached names, explicit fresh-session
delete-only recovery, collision refusal/opt-in replacement and imported inline
values/invalid duplicate or multi-rule rename sources.

Run `python scripts/verify-headless.py --feature multi-node-drafts` for **14 frames**:
actual editor name/description/priority input; destinations; Cancel with retained
parent draft and no commands; rename review; held ACK and independent AskRule;
Stop before original deletion; continuation and exact durable rules on both
nodes; typed invalid/valid file import; changed-file-after-review preservation;
invalid clipboard with no path fallback; captured clipboard changed after review
with only the reviewed rule applied. Two synthetic daemons observe exactly five
commands each. Inspect every recorded PNG; use the final result's process/socket
cleanup flags. The retained first failed compile attempt is not passing proof.

The current six core cases, both 14-frame focused flows and the general 48-frame
regression are indexed in `.verification/multi-node-drafts-result.json`. These
do not complete multi-node export, per-target operand-difference inspection,
new native compositor/picker interactions or the final whole-app UI/UX pass.

## Multi-node folder export

Pinned source `dialogs/events/views.py::on_menu_export_clicked` exports every
node through `rules.py::export_rules`, creating a directory per node containing
individual OpenSnitch JSON rules. Rustleblower's **Export nodes…** offers all
cached nodes, including offline ones, explicit checkboxes and Select all. All
rules are the default scope; a checkbox applies the captured main-page text and
duration filter to every node. Source row marks are deliberately not reused as
foreign-node identities. Cached nodes refresh explicitly without daemon commands.

Choose a parent directory and Review export. One consistent read snapshot copies
selected wire blobs into sequential private disk spools. It releases the live
SQLite connection before JSON encoding/individual-file writes. Each rule is
decoded alone; source data is not loaded into a rule vector or multiplied per
node. The final review retains only an owned temporary directory and at most 32
compact summaries. Later inventory/reconnection changes cannot alter Apply.
The review shows per-node rules, awaiting-observation counts and JSON sizes.

Export reviewed folders copies the frozen files into a new staging directory,
then uses Linux rename with NOREPLACE to publish the complete tree. It does not
merge into an existing folder. A bounded manifest maps unique safe directory
components to exact node names. Rules keep their original names/complete fields
in JSON; node and rule filename components use a readable prefix and full digest
to prevent path traversal, slug collisions and cross-node same-name overwrites.
Import an individual node subfolder through the existing rule importer; the root
manifest is metadata, not a daemon rule.

Cancel/Close sets a shared stop flag, checked per node/record/file and before
publication. A failure or cancellation before atomic rename removes staging;
previous exports remain untouched. A cancellation racing after publication cannot
undo a completed export. Directory/file synchronization follows output writing.
The generic window-manager close still minimizes; use the actual Cancel action.

Bounds: 32 nodes, each 10,000 rules/2 MiB encoded and 8 MiB cumulative owned rule
allocations; one decoded rule/JSON object at once, capped at 256 KiB/512 KiB, and
512-KiB SQLite page cache. Wire spools use at most 64 MiB plus 1,600,000 framing
bytes. JSON is capped at 8 MiB/node (256 MiB aggregate); capture can temporarily
retain both spools and JSON, and publication can retain both captured and staged
JSON. Output has at most 320,000 rule files plus 32 folders and a small manifest.
Path input is capped at 4096 bytes; captured search at 1024 bytes. Catalogue node
strings are checked before copying; oversized blobs are refused before decode.
There is one owned worker job per view and no unbounded progress queue.

Four `tests/export_nodes.rs` cases cover 260 rules over multiple pages, distinct
path-like node names/same rule names, offline proposals, immutable source after
cache deletion, an actual successful live WAL checkpoint while review exists,
full JSON roundtrip, repeated exports preserving the first, filtered empty nodes,
cancel, invalid parent preservation, selection/count/size/identity refusal and
more-than-32/oversized cached-node guards. One unit case removes the second
captured node's file and requires no published/staged output, including the
already-copied first node.

`python scripts/verify-headless.py --feature multi-node-export` records eight
frames with actual toolbar/path/checkbox/review/Cancel/filter/export controls,
252 frozen rules across three nodes, a synthetic permission while review is open,
a live inventory replacement that does not alter the captured export, invalid
destination preservation and successful retry. The offline inventory enters
through the production history boundary; connected inventories use Subscribe.
No policy commands are issued. Permission proof here is during open review, not
a deliberately stalled disk write. The general 48-frame GPU regression covers
existing toolbar/editor/permission controls after the added entry point.

Read `.verification/multi-node-export-result.json` for exact current reports.
Native OS picker/compositor input, actual daemon import/kernel enforcement,
long-duration resource/package proof and the final whole-app visual pass remain
separate requirements. Historical sections retain their original proof scope.

A per-target operand-difference inspector is an optional usability idea; the
pinned UI source does not make it a parity requirement. Existing count review
and complete source JSON remain the implemented inspection controls.

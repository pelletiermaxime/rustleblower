# Application rules

The Application rules tab allows manual recovery after a missed prompt. Catalogue
data lives in SQLite; changes go through the existing daemon notification stream.

## Sub-features and entry points

- `rules-browse`: Application rules tab → searchable cached-node dropdown at the
  left of the name/executable/description filter, matching the History toolbar;
  All/Permanent/Temporary plus independent All/Allowed/Denied/Rejected quick
  action filters; Next/First pages of at most 200 rules. Quick filters affect the
  whole catalogue and captured exports/reviews.
- `rules-scrollbar`: always-visible Kit vertical scrollbar beside the virtual
  list. Actual thumb dragging must reach later rows; the toolbar has space below
  the main tabs.
- `rules-conditions`: Application rules → Conditions → typed Created/name/
  description/node/booleans/action/duration/root-operator/executable predicates,
  plus UI-local last received time and explicit Any operator operand/data tree fields,
  with All/Any groups or a bounded nested numbered AND/OR formula. Apply stages
  the same expression for
  catalogue, selected/full-filter reviews/transfers and multi-node exports.
  Eight 256-byte conditions remain the cap. Daemon/first-seen timestamps are not
  invented; other-page filter persistence is separate.
- `rules-added`: the Added (UTC) column, immediately after Rule name and before
  action/duration/status, shows the daemon-reported creation date.
  Click its header for newest first, then oldest first; Rule name restores name
  order. Sorting covers the full bounded catalogue and follows through pages
  and read-only exports. Missing/invalid creation timestamps display Unknown.
- `rules-action`: select a rule → Allow/Deny/Reject; retain its operator, duration,
  priority and logging settings. Wait for daemon confirmation before updating.
- `rules-enabled`: select a rule → Disable/Enable.
- `rules-create`: New application rule → absolute executable path, optional name
  → Allow always/Deny always. The match targets that executable.
  Creation and editor renaming reject names already present in the confirmed
  catalogue. Import's explicit Replace option remains the overwrite workflow.
- `rules-delete`: select a rule → Delete → Keep rule or Delete rule.
- `rules-editor`: select → Edit…; or New application rule → All rule options….
  Edit metadata, action, preset/custom duration, priority, logging and a flat AND
  group of conditions. Match fields include all installed 1.7.2 operands;
  use Equals/Regular expression/CIDR/watched-directory or an inclusive source/destination port range as appropriate. Match JSON
  roundtrips the operator. Save waits for confirmation. A rename confirms the new
  rule before deleting the old one; partial failure remains visible.
- `rules-duplicate`: select → Duplicate… → change the draft → Save rule. Cancel
  sends no command. Saving retains the original rule.
- `rules-transfer`: Import… → JSON file/directory path or Read clipboard → review
  → Apply reviewed rules. Duplicate names skip by default; the Replace checkbox
  permits overwriting. Export filter… includes all pages; Export selected… uses
  checked names or the highlighted rule. Choose JSON file/folder/clipboard.
- `rules-bulk`: check rows/Select page → action → review → Apply. Names remain
  selected while paging. Clear selection, or change the filter, clears them.
  The review can include every matching rule across all pages. Stop waits for the
  current reply; retry sends only remaining rules. Delete has a review step.
- `rules-copy-nodes`: Copy to nodes… → selected/marked or full-filter source →
  explicit destinations/replacement → review → sequential acknowledged copy.
  Per-node results, Stop/retry and exact source preservation have dedicated
  [coverage and bounds](rule-distribution.md).
- `editor-nodes`: Apply to nodes… captures the current unsaved draft, including
  name/metadata, and reviews explicit destinations. Cancel retains parent fields.
  Renaming confirms the new name before deleting the old one per destination;
  Stop/continue retains the removal stage.
- `rules-import-nodes`: Import to nodes… reads bounded file/directory/captured
  clipboard data into the same immutable review. Explicit replacement, exact
  source values and per-node results apply.
- `rules-export-nodes`: Export nodes… selects cached nodes, all rules or the
  captured filter, then captures immutable disk-backed data for count review.
  Export reviewed folders publishes a new complete directory tree, with
  importable node subfolders and identity manifest. Cancel and pre-publication
  errors leave no partial destination. Offline caches are supported.
- `rules-errors`: daemon error preserves the old catalogue value; timeouts and
  disconnects explain that confirmation is unknown and require a reload.
  A daemon ERROR can follow an in-memory change before a disk failure, so a
  persistent warning identifies cached entries as the last confirmed values.
  A fresh inventory clears that warning; do not claim rejected commands prove
  unchanged live policy.
- `rules-popup-observation`: lasting popup replies appear labelled awaiting
  observation with actions disabled. A daemon Ping containing that rule confirms
  its reported values; self-generated decision history does not. Exports include
  proposals, but mutation reviews/current-rule editing require confirmed entries.
- `rules-refresh`: Refresh connection → request a fresh Subscribe inventory.
  Refuses pending decisions/commands and keeps the owned socket inode unchanged.
  Core protocol proof covers guards and replacement; native refresh input remains
  unverified. This deliberately reconnects the notification stream.
- `rules-bounds`: 10,000 rules / 2 MiB per inventory, 64 KiB encoded per rule,
  16 pending commands / 8 outbound queued messages. Current inventory readiness
  and session identity gate edits; stale replies cannot modify a replacement.

## Verification

The focused `python scripts/verify-headless.py --feature browsing` journey checks
quick action filters across 200+20 matching rules, combined Permanent scope and
actual pointer dragging to row 200. Its two-node History fixture also exercises
node selection and list/table scrollbars. See `.verification/browsing/result.json`
for the current evidence and boundaries.

Run `python scripts/cargo.py test --no-default-features --locked`. The rule protocol
tests exercise a real UI-side gRPC server, daemon command/reply streams, catalogue
queries and SQLite durability. They cover action rejection/confirmation, preserved
operators, enable/disable/delete and reconnect cancellation. Unit tests check
pending-operation capacity, expiry, inventory readiness and filename validation.

Run `python scripts/verify-headless.py`. The expanded flow uses actual Kit tab,
ListItem, Input and Button events against an isolated gRPC command fixture:

1. Open Application rules; inspect `08-application-rules.png`.
2. Next/First; select a rule and allow it; inspect `09-rule-allowed.png`.
3. Create a permanent rule; Ctrl+F and type its executable; inspect `10-rule-created.png`.
4. Reject a daemon command, require the old action and visible error;
   inspect `11-rule-rejected.png`.
5. Cancel deletion, then confirm it; inspect `12-rule-deleted.png` and the empty page.
6. Edit a source rule, rename, set description/priority/logging/duration, and add
   a case-insensitive host regex; inspect `13-rule-editor.png` and
   `14-rule-edited-draft.png`. Require acknowledged ChangeRule then DeleteRule,
   the updated persisted values and editor closure/focus restoration.
7. Cancel a duplicate (no command), reopen it, roundtrip Match JSON, and try an
   invalid custom duration (no command). Enter `90s` and save. Require two rules,
   original unchanged and copied operators equal; inspect `15-rule-duplicate.png`,
   `16-rule-match-json.png`, `17-rule-invalid-duration.png`.
8. Export the filter to file/clipboard; import that file with duplicate skipping;
   import adjacent JSON from the clipboard, fail the second command, then retry
   only the remaining rule. Inspect `18-rules-export.png`, `19-import-skipped.png`,
   `20-import-review.png`, `21-import-partial-failure.png`.
9. Mark a row/Select page; review Disable; hold its daemon reply and answer a
   separate permission popup, then Stop and resume. Review Delete, close it
   without commands, reopen and confirm it. Inspect `22-bulk-selection.png`,
   `23-bulk-stopped.png`, `24-bulk-delete-review.png`, `25-prompt-during-bulk.png`.
10. Explicitly replace an existing name. Select 201 names across two pages and
    export exactly that subset. Then apply Disable to all 229 remaining matches
    and require later-page persistence; inspect `26-bulk-all-pages.png`.
11. Answer a permanent popup; inspect `37-popup-rule-awaiting.png` for its label
    and disabled edits. Report that rule through the fixture's Ping and require
    the daemon-reported metadata plus `38-popup-rule-observed.png`.

`tests/rule_transfer.rs` and the snapshot unit test separately verify 260-rule
file/folder/clipboard roundtrips, filtering/selection, snapshot consistency under
concurrent writes, filename suffix collisions, existing destination preservation,
JSON/object/count limits and import formats. Native platform file pickers are
not yet input-tested; the GPU journey types paths and uses the real clipboard API.

The Python demo fixture supplies six application rules by default (`--rules N`
changes that) and processes commands in memory. `--reject-rule-changes` enables
the error path. It never changes real firewall rules. Existing native history
and prompt checks still use owned instances, but rules input itself currently
has background GPU proof, not compositor/native-input proof.

## Remaining parity work

Richer keyboard navigation and per-node targeting remain tracked
in `docs/port-plan.md`. Do not imply those are implemented. Confirmed daemon
responses prove the protocol outcome, not independent kernel enforcement.
The history feature covers creation from a recorded executable/destination and
current matched-rule lookup, including existing-name protection in both editors.
Background input covers representative editor fields; CIDR/list/environment/
checksum and every field variant still need dedicated input proof. Core JSON
tests cover timestamps, legacy list encoding, omitted protobuf flags, safe
field/type pairs and custom duration units. The original daemon's flat AND
serialization limitation is preserved explicitly; nested groups cannot be saved.


## Inclusive port ranges

The structured match-type picker offers **Port range (inclusive)** for Source
port and Destination port. Enter `min-max`, e.g. `4000-4010` and `8000-8080` in
separate AND conditions. Reversed/missing/negative/signed/overflow/nondecimal
bounds are refused before sending. The pinned daemon removes ASCII spaces only,
then parses unsigned 64-bit decimal bounds with `min <= max`; Rust validates the
same grammar and retains the original text through JSON/structured switching,
acknowledged storage, duplication/import/export and edits. It does not expand
ranges into regular expressions. Actual ports are 0–65535; imported uint64 bounds
outside that domain remain intact to match the daemon's wider numeric grammar.
Other match fields are refused for this port-specific UI type.

All ChangeRule/EnableRule submissions, including imports, quick actions and
bulk actions, pass the same current-session compatibility guard. Version 1.9+
establishes support; an existing range rule in Subscribe also establishes a
backport. An older/unknown/ambiguous version with no reported range refuses the
command without queueing. Disable/Delete remain available. Capabilities reset
when a connection is replaced. The guard does not infer support merely from an
import file, our own popup reply or protobuf/schema equality. The operator was
introduced by e1b961ae while master still reported 1.8.0; an arbitrary 1.8 build
cannot be identified from that string alone. Explicit compatibility selection
for otherwise unconfirmed backports remains pending.

`python scripts/cargo.py test --no-default-features --locked --test port_ranges`
adds three cases: 21 grammar/uint64 fixtures and exact JSON roundtrip; real gRPC
early Ping/version, held ACK/no optimistic persistence, independent AskRule and
ERROR preservation with complete SQLite export; old/unknown/malformed versions,
empty outbound stream, same-connection replacement and observed backports.
Set `RUSTLEBLOWER_RANGE_EVIDENCE` to a fresh absolute directory to save immutable
Rust fixtures, then run `python scripts/verify-port-ranges.py --fixtures /absolute/fixtures --evidence /absolute/new/proof`.
The helper extracts only unmodified `compileRange` and `rangeCmp` bodies from the
pinned source into ignored evidence, compares all 21 grammar cases and inclusive
boundary matches using local Go with module networking disabled, and links no
daemon/interception/RPC packages. This establishes parser/matcher equivalence,
not real-daemon capability or kernel enforcement.

`python scripts/verify-headless.py --feature port-ranges` drives eight GPU frames:
invalid bounds, unknown capability refusal, Match JSON roundtrip, held ACK,
independent permission input, confirmed range persistence, rejected-draft
preservation and exact clipboard export/import. It uses actual Kit Select/Input,
button and scroll events. The editor scroll body uses Kit's official test-only
observation hook; root assertions read through Kit Root. This is background
GPU/input/protocol/storage proof. Native compositor range editing remains pending.

## Typed catalogue search extension

Read [advanced rule search](../../../../docs/advanced-rule-search.md) for the
pinned source contract and limits. Conditions uses actual Kit Select/Input,
Checkbox and Button controls with an observed scroll body for input proof.
The editor stages up to eight 256-byte predicates, All/Any groups and an optional
bounded nested AND/OR formula;
Apply preserves the current quick search/scope and clears stale row selection.
Old node/session editors cannot apply to a replacement session.

One prepared native predicate serves catalogue keyset pages, selected/full-filter
transfer/review and each actual node in multi-node capture. It streams candidates
within the existing 10k-rule/2-MiB wire cap and returns 200 matches with a bounded
lookahead and 8-MiB decoded-page cap. It never passes SQL/expression text through
or rewrites cached protobuf bytes. Existing unknown operator strings/data and
pending-observation/stale snapshot protections remain intact.

`tests/rule_search.rs` covers typed fields/negation/prefix/suffix/inclusive
boundaries, literal/injection strings, group/value caps, matching pages and
selected/full-filter consistency, unknown cached wire preservation, immutable
multi-node capture and pending review refusal. The predicate module has a grouped
multi-page snapshot unit case with an actual later-page writer change. All eighteen
focused integration cases passed (four rule search, five existing rules, five
rule transfer and four multi-node export), and the grouped snapshot unit case passed. Exact commands,
logs, namespace proof and source hashes are preserved in
`.verification/rule-search-implementation-20261017T023808Z/result.json`.
The focused `--feature rules-advanced` journey passes eight background GPU/input
frames at `.verification/headless/20261017T024640Z-55e71c6e/result.json`: actual
invalid boolean refusal, action/enabled plus description/inclusive-created
groups, root/inner All/Any, exact catalogue rows, staged Close/reopen, matching
whole-wire JSON export, removal and clear. Process/socket cleanup passes; scoped
hashes and boundaries are in `.verification/rules-advanced-result.json`.
Native compositor and other-page presentation persistence remain separate.
Explicit Any operator fields search the bounded full tree; root-only fields retain
their existing semantics and no daemon/first-seen timestamp is invented.

## Nested formula input proof

The expanded focused journey passes ten real-GPU frames in
`.verification/headless/20261017T032547Z-ab61a107/result.json` (51.60 seconds).
Duplicate/out-of-range and SQL-like formula Apply preserve the prior catalogue.
Actual reordered `4 OR (1 AND (3 OR 2))` changes the exact rows and full protobuf
JSON export to four matches. Reopened actual Input copies that exact expression;
root group clicks remain inert while formula is present, then Add/Remove
reactivate group toggling. Staged Close preserves the applied formula, and all
previous group/partition/Clear assertions pass. Invalid feedback, numbered
conditions and export states were visually inspected. Full source/image hashes,
core parser/integration and compile/lint boundaries are recorded in
`.verification/search-formula-result.json`. This formula grammar does not change
the separate nested-operator serialization limitation above.

## Received time and nested field input proof

The focused rules journey now passes fourteen real-GPU frames in
`.verification/headless/20261017T035229Z-9481b893/result.json` (77.16 seconds).
All ten formula/group regressions remain. Actual Last received by UI date input
refuses an invalid date without changing the prior six rows; At least 0 includes
the received fixtures. Exact Any operator operand `dest.host` combined with Any
operator data containing `leaf.deep` finds only the deeply nested fixture.
Actual full-filter export equals its complete JSON document, including siblings;
Does not contain excludes the entire matching tree and retains five others.

Read-only export intentionally retains nested cached wire. Mutation Import still
refuses that nested group and the test explicitly asserts this boundary. The
initial verifier's mistaken use of the mutation importer is retained in the
failed attempt. `.verification/rule-receipt-tree-gpu-result.json` links exact
134-file rendered source, PNG/export hashes, immutable binary and the separate
31-case core checkpoint. Six unused preference-view warnings existed before its
caller integration; this compile is not strict-Clippy evidence. Images 11/12/14
were visually inspected with readable controls and result counts. Native
compositor, policy mutation/enforcement and packaging remain separate.

## Added date column

The Added (UTC) column uses the daemon rule's creation timestamp, with Unknown
for absent/invalid dates. The first header click sorts newest first, the second
oldest first; Rule name restores name order. This reuses the bounded background
global ordering implementation, so Next/First pages and captured read-only
exports share the selected order. No database schema or daemon policy change is
needed.

The focused rules-advanced journey now passes seventeen real GPU/input frames
in `.verification/headless/20261017T051318Z-e77f969d/result.json`. Three added
frames verify header input, exact date ordering and header/cell alignment; the
existing fourteen filter/tree/export frames remain. The existing 650-rule
global sorting/paging/export regression, strict all-target headless Clippy,
catalogue check and normal application build pass. Exact source/evidence hashes
are in `.verification/rule-added-result.json`; fixtures do not change the host
daemon or rules.

The subsequent column-order refinement places Rule name, Added and
Action/Duration/Status in separate aligned columns. The same seventeen-frame
journey passes in `.verification/headless/20261017T052052Z-ad1f79e1/result.json`
(82.99-second runtime), including an assertion that status space follows Added.
The current scoped report is `.verification/rules-column-order-result.json`.

### 2026-10-17 browsing checkpoint

The seven-frame GPU journey passed at
`.verification/headless/20261017T060246Z-c979b303/result.json`; all requested
filter and pointer-drag assertions passed. Eighteen focused storage/catalogue
tests and eight localization tests passed at the recorded checkpoints. Later
major dependency upgrades in the shared checkout caused whole-project API/type
compile errors; this checkpoint does not establish a successful current normal
desktop build. The aggregate retains compiler logs and exact boundaries in
`.verification/browsing/result.json`.

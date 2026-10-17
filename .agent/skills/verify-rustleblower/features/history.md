# Connection history

Users browse recent daemon reports, filter by executable/destination and action,
and page backward without loading the entire database into memory.

## Sub-features

- `history-node`: use the searchable node dropdown beside search. All nodes is
  the default; cached offline nodes remain available. Exact node scope combines
  with search/action/advanced conditions, persists across restart and follows
  paging, selection and export. Discovery is one bounded background query.
- `history-search`: filter the executable or displayed destination.
- `history-actions`: choose All, Allowed, or Denied.
- `history-reject`: choose Rejected separately from Denied.
- `history-conditions`: Filters, columns & export… → Filters; combine up to eight
  typed conditions across the general columns, working directory and rule metadata.
  Choose All/Any between groups and within each group, or a bounded nested
  AND/OR formula referencing each numbered condition once.
  Apply alongside the quick search/action filter; invalid values stay staged.
- `history-table`: Columns → Use table view → choose columns → Save history view.
  Pinned Kit DataTable virtualizes the shared 200-row page; headers sort all matches.
  Column widths, header sort and applied search/action/conditions survive restart.
- `history-preferences`: reviewed days/disk-target/idle cleanup and persisted display refresh;
  [retention preferences](history-preferences.md) has the full proof contract.
- `history-export`: Export → absolute destination → CSV/JSON Lines → Export all
  matching events. Confirm replacement explicitly. Clipboard copies one matching
  page as CSV. Search/action/sort/page are captured on opening; staged conditions
  and chosen columns also apply. Reopen to capture changes from the main view.
- `history-scrollbars`: always-visible Kit vertical scrollbars in the default
  list and table view; the table also has a horizontal scrollbar when columns
  overflow. Toolbar spacing starts below the main navigation.
- `history-pages`: Older freezes an earlier page; Latest resumes following reports.
- `history-selection`: row checkboxes mark events independently of Details;
  Shift-click replaces an inclusive range, Select all matching captures the
  query, and individual unchecks exclude rows. Older/First page preserve the
  selection and arrival ceiling; Latest or changed filters/sort clear it.
- `history-selected-output`: open tools after marking, choose Export, data
  columns and CSV/JSON Lines, then Export selected events or Copy selected events.
  Both use the captured selection, including rows on other pages.
- `history-persistence`: history survives restart and duplicate reports deduplicate.
- `history-errors`: queue/storage loss is visible as incomplete history.
- `history-details`: select a row → Details… → Overview/Arguments/Matched rule;
  copy metadata or command line. One detail snapshot is loaded on demand.
- `history-rule`: Application rule… or Application + destination… opens the full
  editor; saving waits for the daemon. Open current rule… reads the current
  catalogue entry rather than editing the historical summary.

The complete captured-selection journey now uses GPUI Kit's test runner. See
[headless testing](../../../../docs/headless-testing.md) for the suite's JSON
checkpoints and the separate native/pixel proof boundary.

## How to get to it (user POV)

- Open the main Rustleblower window; history is its main content.
- Click the search field or press Ctrl+F, then type a filter.
- Choose All/Allowed/Denied or use Older/Latest below the list.
- Select a row and choose Details…; use the tabs, copy buttons and rule actions.

## Driving it with GPUI input and native controls

Preconditions: the default synthetic fixture supplies 430 events on an isolated database.

- **Main browsing controls.** `python scripts/verify-headless.py --feature browsing`
  uses its own two-node history and 660-rule fixture. Require actual offline-node
  selection, combined action filtering, visible toolbar spacing and pointer-drag
  movement to row 200 in both list views and movement in the History table.
  Require saved node/table state after production Quit and owned socket cleanup.

- **Pages/action filter.** Run `python scripts/verify-headless.py`. It clicks
  `older`, `latest`, and `deny`, asserting the resulting rows. Inspect JSON checkpoints 01–04.
- **Keyboard search.** That flow presses Ctrl+F and types a missing query, then
  Ctrl+A and `firefox`. Require the empty state and matching denied rows.
- **Foreground search.** Run `python scripts/verify-native.py run --feature history`.
  Require OCR of No matching events after Ctrl+F/type, populated/filtered PNGs,
  successful fixture RPCs, and owned cleanup.
- **Persistence and limits.** Run `python scripts/cargo.py test --no-default-features --locked`.
  Storage checks reopen the file, verify deduplication/filtering and page sizes,
  and expire old rows. The large-record check verifies retained data/files fit
  its 16 MiB test target.
- **Filters/columns/export.** Run `python scripts/verify-headless.py --feature history-tools`.
  Require sixteen JSON checkpoints and actual filtered rows/files/clipboard assertions. The flow
  checks 144 Rejected rows, combines UID=1001 and command Contains blue1 for 108
  matches, rejects port 65536, saves/reopens table/UID/command columns, clicks the
  actual header through descending then ascending, exports all 108 rows as CSV
  and JSON Lines, refuses an existing file, copies CSV and pages sorted history.
  Output files remain alongside evidence. The full default flow is separate.
  Frames 11–16 add actual search/action/condition input, a real Kit pointer drag
  of the time-column edge, immediate Quit flush, a fresh backend/GPUI restoration
  of painted controls/widths and exact 72 matches, stopped writes after an external
  edit, an independent real AskRule/Deny once, and explicit owned-file repair
  followed by another cold restoration. Old cursors/ceilings are not persisted.
- **Native conditions.** Run `python scripts/verify-native.py run --feature history-tools`.
  Ctrl+F/Tab/Space opens tools. Enter opens the pinned Select; choose UID and
  Equals, type 9999, Add then Apply. Require main-window No matching events OCR,
  retained 430 fixture rows and all cleanup booleans. Allow rendered frames between
  focus/commit keys. Native pickers, column/export buttons, width dragging and
  horizontal scrolling remain outside this proof.
  Also independently reopen the version-3 settings JSON and require exactly UID
  Equals 9999, 430 retained rows, zero policy commands and complete cleanup.
  This checks native input plus durable filter saving;
  cold restoration and width dragging require separate native checks.
- **Details and rule creation.** The background flow inspects Overview, Arguments
  and Matched rule, copies metadata/command line, opens the current rule without
  saving, then confirms a permanent application+destination draft. Inspect
  `27-connection-overview.json`, `28-connection-arguments.json`,
  `29-connection-matched-rule.json`, `30-current-matched-rule.json`,
  `31-rule-from-history.json`. Require source/destination, PID/UID, cwd and argument
  assertions plus the confirmed rule's actual operator fields in SQLite.
- **Limits and naming.** That flow rejects a duplicate new name in the full editor
  and quick form without sending a command (`32-history-rule-name-collision.json`,
  `34-quick-rule-name-collision.json`). It ingests oversized arguments, captures
  `33-limited-connection-details.json` and requires the limit in copied details.
  Core tests separately cover schema migration, restart persistence, UTF-8/argument
  limits, refusal to seed incomplete matches, map-order-independent replay
  deduplication, and detail cleanup alongside retention.

## Gotchas

- The new selection controls have seven focused core/export checks and concrete
  native compilation; focused Kit input/layout proof is pending. Read
  [captured-selection contract](../../../../docs/history-selection.md) for exact
  descriptor/export limits and pruning semantics. A ceiling excludes later
  arrivals; it does not pin rows against retention.

- Pages contain at most 200 rows; only visible rows render. UTC labels differ
  from local desktop time. The disk target is not an external filesystem quota.
- Quick search applies to displayed executable/destination strings. Typed filters
  also query bounded metadata. Bounded arguments
  and connection metadata are stored separately; environment/checksum/process-tree
  history and arbitrary historical rule conditions are excluded.
- Schema-1 entries have summaries only. New snapshots are limited to 24 KiB;
  incomplete match fields cannot seed rules. UID/PID are the reported numbers.
- Details/copy/current-rule/draft flows currently have background Kit input/layout and
  core persistence proof. Native compositor input for these actions, local
  UID-name resolution and live process inspection remain pending.
- Original NULL legacy metadata and the new cwd metadata revision backfill old
  detailed snapshots in 32-row background batches. Per-row schema markers allow
  restart/resume without redoing completed chunks, retaining indexed/unknown JSON
  fields and adjusting byte estimates by the serialized-size delta. This extends
  JSON metadata without changing SQLite schema 9. Indexing feedback remains visible; missing
  fields are blank/NULL, not UID zero. Applying metadata filters and sorting
  metadata columns wait for indexing. The actual worker query path rejects metadata
  predicates/sorts while old metadata remains. A dedicated unit case exercises an
  actual 32-row chunk followed by cursor-reset resume, readiness and byte accounting.
- Sorted pages use value/ID cursors and a fixed arrival ceiling; retention may
  remove rows while browsing. Latest releases the ceiling. Column selection/mode and display refresh
  persist in a version-3 16-KiB file with search/action/conditions/sort/widths.
  Versions 1/2 migrate in memory without dropping column/refresh choices. One
  background writer and one pending snapshot coalesce changes after 500 ms;
  close/Quit request an immediate flush. Quit's existing 500-ms limit covers both
  window and history writers after backend/permission shutdown. There are at most
  eight 256-byte conditions, up to eight nonempty two-level groups and twenty finite,
  unique 40–2000 px column widths.
  UI-side save errors are session-only and stop automatic writes. Invalid startup
  settings use defaults with a warning, preserve the file and never prevent RPC
  startup. Repair/remove only the owned settings file and restart to recover.
  All column/refresh/presentation writers merge their own fields under one lock
  and refuse stale/oversized/symlink settings. Hash checks are not filesystem CAS.
- File export uses a private backup (15-second acquisition limit), finishes any
  legacy metadata there, then streams selected displayed values. Source cap is
  4 GiB, private snapshot cap 8 GiB, output cap 256 MiB, clipboard cap 4 MiB.
  Temporary snapshot/sort disk use is additional to retention. Stop/failure keeps
  the old target, and database/WAL/SHM/preferences paths are reserved.
  Actual cap-sized output, native stop/mid-export close and OS picker behavior
  remain unverified; core tests prove cancellation/file preservation and concurrency.
- Search-field pointer focus, Allowed/All variants, and the incomplete-history
  banner currently have source-backed entry points beyond the named UI flow.
- Error counters are per run; this history is not a lossless audit log.

## Advanced typed search extension

Reference and exact limits: [advanced history search](../../../../docs/advanced-history-search.md).
The pinned source adds cwd, not-contains, starts/ends-with, inclusive comparisons
and compound expressions. The native operator/field controls implement these
through fixed SQL templates and bound values. `%`, `_`, quotes and SQL-looking
strings are literal text. ASCII case folding follows existing SQLite behavior;
Exact match remains case-sensitive. Cwd is bounded to 4096 UTF-8-safe bytes, with
empty/unavailable values treated as unreported. The optional numbered formula
now supports nested AND/OR within the eight-condition budget; general SQL and
unary NOT are outside this grammar.

The connection-details Inspect process action opens current daemon process data
for the recorded PID. It keeps the event's node, rejects a queued stale session
and warns about PID reuse; see [Live inspection](live.md#history-to-process-inspection)
and `--feature history-process` for focused input/protocol proof and its boundaries.

Run `python scripts/verify-headless.py --feature history-advanced` for typed/grouped/formula
search and matching output. Run `python scripts/verify-headless.py --feature history-selection`
for cross-page marks, captured output, clipboard equality and independent permission input.

# Connection history

Users browse recent daemon reports, filter by executable/destination and action,
and page backward without loading the entire database into memory.

## Sub-features

- `history-search`: filter the executable or displayed destination.
- `history-actions`: choose All, Allowed, or Denied.
- `history-reject`: choose Rejected separately from Denied.
- `history-conditions`: Filters, columns & export… → Filters; combine up to eight
  typed AND conditions across the general columns, duration/logging/priority/reason.
  Apply alongside the quick search/action filter; invalid values stay staged.
- `history-table`: Columns → Use table view → choose columns → Save history view.
  Pinned Kit DataTable virtualizes the shared 200-row page; headers sort all matches.
  Widths survive page refresh within the current view, but are not persisted.
- `history-export`: Export → absolute destination → CSV/JSON Lines → Export all
  matching events. Confirm replacement explicitly. Clipboard copies one matching
  page as CSV. Search/action/sort/page are captured on opening; staged conditions
  and chosen columns also apply. Reopen to capture changes from the main view.
- `history-pages`: Older freezes an earlier page; Latest resumes following reports.
- `history-persistence`: history survives restart and duplicate reports deduplicate.
- `history-errors`: queue/storage loss is visible as incomplete history.
- `history-details`: select a row → Details… → Overview/Arguments/Matched rule;
  copy metadata or command line. One detail snapshot is loaded on demand.
- `history-rule`: Application rule… or Application + destination… opens the full
  editor; saving waits for the daemon. Open current rule… reads the current
  catalogue entry rather than editing the historical summary.

## How to get to it (user POV)

- Open the main Rustleblower window; history is its main content.
- Click the search field or press Ctrl+F, then type a filter.
- Choose All/Allowed/Denied or use Older/Latest below the list.
- Select a row and choose Details…; use the tabs, copy buttons and rule actions.

## Driving it with GPUI input and native controls

Preconditions: the synthetic fixture supplies 430 events on an isolated database.

- **Pages/action filter.** Run `python scripts/verify-headless.py`. It clicks
  `older`, `latest`, and `deny`, asserting the resulting rows. Inspect PNGs 01–04.
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
  Require ten PNGs and actual filtered rows/files/clipboard assertions. The flow
  checks 144 Rejected rows, combines UID=1001 and command Contains blue1 for 108
  matches, rejects port 65536, saves/reopens table/UID/command columns, clicks the
  actual header through descending then ascending, exports all 108 rows as CSV
  and JSON Lines, refuses an existing file, copies CSV and pages sorted history.
  Output files remain alongside evidence. The full default flow is separate.
- **Native conditions.** Run `python scripts/verify-native.py run --feature history-tools`.
  Ctrl+F/Tab/Space opens tools. Enter opens the pinned Select; choose UID and
  Equals, type 9999, Add then Apply. Require main-window No matching events OCR,
  retained 430 fixture rows and all cleanup booleans. Allow rendered frames between
  focus/commit keys. Native pickers, column/export buttons, width dragging and
  horizontal scrolling remain outside this proof.
- **Details and rule creation.** The background flow inspects Overview, Arguments
  and Matched rule, copies metadata/command line, opens the current rule without
  saving, then confirms a permanent application+destination draft. Inspect
  `27-connection-overview.png`, `28-connection-arguments.png`,
  `29-connection-matched-rule.png`, `30-current-matched-rule.png`,
  `31-rule-from-history.png`. Require source/destination, PID/UID, cwd and argument
  assertions plus the confirmed rule's actual operator fields in SQLite.
- **Limits and naming.** That flow rejects a duplicate new name in the full editor
  and quick form without sending a command (`32-history-rule-name-collision.png`,
  `34-quick-rule-name-collision.png`). It ingests oversized arguments, captures
  `33-limited-connection-details.png` and requires the limit in copied details.
  Core tests separately cover schema migration, restart persistence, UTF-8/argument
  limits, refusal to seed incomplete matches, map-order-independent replay
  deduplication, and detail cleanup alongside retention.

## Gotchas

- Pages contain at most 200 rows; only visible rows render. UTC labels differ
  from local desktop time. The disk target is not an external filesystem quota.
- Quick search applies to displayed executable/destination strings. Typed filters
  also query bounded metadata. Bounded arguments
  and connection metadata are stored separately; environment/checksum/process-tree
  history and arbitrary historical rule conditions are excluded.
- Schema-1 entries have summaries only. New snapshots are limited to 24 KiB;
  incomplete match fields cannot seed rules. UID/PID are the reported numbers.
- Details/copy/current-rule/draft flows currently have background GPU/input and
  core persistence proof. Native compositor input for these actions, local
  UID-name resolution and live process inspection remain pending.
- Schema 4 backfills old detailed snapshots in 32-row background batches, with
  NULL markers for restart/resume. Indexing feedback remains visible; missing
  fields are blank/NULL, not UID zero. Applying metadata filters and sorting
  metadata columns wait for indexing. Mid-backfill interruption/restart has a
  source-backed entry point; automated migration proof covers completed backfill.
- Sorted pages use value/ID cursors and a fixed arrival ceiling; retention may
  remove rows while browsing. Latest releases the ceiling. Column selection/mode
  persist in a 4-KiB file; widths, header sort and conditions are not persisted yet.
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

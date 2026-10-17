# Statistics

## User behavior

Open Statistics or press Ctrl+3. Browse Applications, Hosts, Addresses, Ports,
Users or Protocols. Search filters cached values; node scope includes cached
nodes, not only the current session. Sort by hits or name. Show chart displays
the top ten. Next/Previous page uses a frozen snapshot; Latest resumes reports.
Select a value to show exact matching history or reset its local baseline.
Reset category covers all values for that category/node scope, ignoring search.
Both require confirmation. Only a selected reset can additionally delete history
through an explicit checkbox. Export captures all matches as CSV/JSON Lines or
copies a 200-row page; existing files require explicit replacement.

Row checkboxes additionally mark values independently of the focused
history/reset value. Shift-click replaces a range and Select all matching
captures the immutable counter snapshot. Marks survive Previous/Next page;
Latest or category/node/search/sort changes clear them. Export selected values
and Copy selected values use the captured marks and chosen data columns/format.
The export window must be reopened after changing marks.

## Source contract

The daemon sends absolute cumulative maps independently of history and can evict
keys. See pinned `daemon/statistics/stats.go` and installed UI `service.py` /
`dialogs/stats.py`. Replayed reports do not accumulate, omitted keys retain their
last report, raw decreases clear the per-key baseline, and lower uptime starts
that node's new lifetime. Local resets never reset the daemon or send policy
commands. A bounded cache is not a permanent traffic audit or byte counter.

## Entry points and proof

- Core: `python scripts/cargo.py test --no-default-features --locked`. Statistics
  integration tests use real gRPC on disposable sockets; module tests enforce
  key/byte/node/age and allocation bounds. The contention test holds SQLite's
  writer, coalesces 20 reports and answers a permission before the lock is freed.
- Background: `python scripts/verify-headless.py --feature statistics`. Require
  ten PNGs, real GPU, scoped socket removed and owned process exited. Actual Kit
  input changes tabs, node/sort selectors, chart, frozen pages and latest;
  independently read files/clipboard and SQLite confirm all-page output and
  baseline/history changes. The RPC fixture reports totals larger than history.
- Native: build desktop **after** core tests, doctor, then
  `python scripts/verify-native.py run --feature statistics`. Require painted
  totals/applications, empty-state OCR after keyboard search, three persisted
  application counters / 430 hits, zero daemon commands and every cleanup flag.
- Selected output: three new model/persistence/export cases and the four
  existing protocol regressions pass, with source/evidence boundaries in
  [counter-selection contract](../../../../docs/statistics-selection.md).
  The linked controls pass the concrete all-features/all-targets native check;
  the expanded fourteen-frame GPU/input journey below proves checkbox/range
  selection, chosen-column output and the observed scrolling export body.

## Screenshots

1. Applications / totals / bounded first page.
2. Pinned Kit top-ten BarChart with exact labels.
3. Protocol category after visiting the other five categories.
4. Frozen second page while newer counter reports arrive.
5. Existing CSV target preserved after refusal.
6. JSON Lines export and copied page confirmation.
7. Exact selected-value/node history drilldown.
8. Selected-reset confirmation and explicit history-removal checkbox.
9. Confirmed selected reset / removed matching history.
10. Category reset for scoped node, with the search still visible.
11. First counter mark in the initial200-row page.
12. Shift range across pages with201selected values and frozen counter201.
13. Frozen349-row selected output, exactly Value/Hits fields and identical clipboard.
14. Independent permission response with selected counter output open.

## Limits and remaining proof

10,000 keys / 2-MiB reports reserve from shared 8-MiB ingestion. Persistent data
is capped at 32 nodes / 10,000 keys / approximately 2 MiB of counter rows, with
retention during reports. One latest slot per node (at most 32) coalesces pressure; statistics health
is separate from event loss. Pages retain 200 rows, offsets and a top-ten preview;
one capture permit / 512-KiB SQLite cache / 4-MiB private snapshot. At most three
view snapshots may coexist across pending/current/export. CSV/JSON output is
16 MiB, clipboard 4 MiB, buffers 16 KiB. Cache data reserves from history's
quarter-target logical budget; files/WAL are not an OS quota.

Native chart/reset/history checkbox/export/picker actions, cap-sized output,
long-running RSS and real daemon enforcement remain unverified. Concurrent named Unix sessions are now supported; remote TCP/TLS and peer
identity remain pending (see Nodes). No verification changes
the system daemon, active socket or firewall. Preserve failure evidence.

## Captured counter selection input proof

`python scripts/verify-headless.py --feature statistics` now passes14 actualGPU frames in `.verification/headless/20261017T043959Z-14963128/result.json` (38.65seconds runtime), retaining the original10 category/chart/filter/paging/export/drilldown/reset observations. Four addedframes exercise pointer/Shift marks across the200-row boundary,201selectedrange clipboard rows, Selectall withone exclusion, and349 frozen selectedJSONL rows with only Value/Hits fields. File/clipboard bytes match; the old201counter stayscaptured despite a later999report. A realDeny succeeds while outputtools areopen. Focusedreset deletes its exacthistory and preserves the newlyintroduced unrelatedpermission event. Aggregate `.verification/statistics-selection-gpu-result.json` retains149-source manifest, immutablebinary, allimages/outputartifacts and7core evidence with explicit observation/dependency/backup-safety source deltas. Native compositor/package/RSS and realdaemon enforcement remain separate.

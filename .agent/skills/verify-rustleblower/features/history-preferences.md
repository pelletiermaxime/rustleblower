# History retention and display preferences

## Sub-features

History → History preferences… opens Storage & retention and Display refresh.
Retention days are 1–365, database target 16–4096 MiB and idle cleanup 1–60 minutes.
Review validates staged values, displays before/after settings and currently
expired event/alert/counter counts, and explains possible additional size eviction.
Back changes nothing; Apply saves settings and prunes within one transaction.
Cleanup never deletes cached application rules or node settings. Reload saved
values discards the staged form. Copy database path copies the current file path.
Database relocation remains a separate operation, not an implicit live migration.

Display refresh is 0 (manual) or 1–60 seconds. Save display refresh applies
immediately and persists separately from retention. Manual mode leaves ingestion,
permission replies, searches, paging, Latest and Refresh now working. New arrivals
do not replace the visible page until an explicit action. Enabling automatic
refresh resumes latest-page updates at the chosen real interval. Older pages
retain their captured arrival ceiling. This setting does not change daemon Ping.

## How to get to it

Use the History preferences button beside existing history tools. The retention
window initially focuses days. Ctrl+Enter reviews and Ctrl+Shift+Enter applies
only after review. Native keyboard: Ctrl+F in History, Tab to existing tools, Tab
to preferences, Space; replace days, review and apply. Existing tools retain their
previous focus order. Readonly database location and copy remain available.

## Source and state contract

Installed OpenSnitch 1.7.2 `config.py` database/max_days, purge_interval and
statsDialog/refresh_interval, `dialogs/preferences.py` database settings and UI
refresh controls, and `service.py::_start_db_cleaner` define this slice's source.
Rustleblower keeps its disk-backed WAL design and bounded days/size controls.

Schema 8 adds one SQLite history_preferences row: days, MiB, cleanup minutes,
revision and whether explicitly edited. Saved UI choices survive restart. Launch
values initialize an unedited database; explicit --history-days/--history-mib
flags override and save those values. Implicit CLI defaults preserve edited UI
settings. The native footer reads effective values rather than stale launch args.
The CLI preserves an owned socket on failure and never replaces another endpoint.

Apply checks the reviewed revision, prunes and saves the new row in the same
transaction, then publishes runtime settings. A failed save rolls back deletion.
A stale editor fails explicitly and cannot overwrite a later edit. Idle cleanup
runs without incoming reports; normal writes still enforce size/age. Errors after
commit describe saved settings and pending physical maintenance, never a rejected
save. The database target is not a filesystem quota: free pages compact gradually,
WAL/headroom and retained non-history caches can keep files above a smaller target.
The report distinguishes logical cleanup from allocated pages and pending caps.

The history view document now uses version 3 (16 KiB), reading versions 1/2
with defaults for missing refresh/presentation fields. Column, refresh and
presentation saves merge only their own fields under the shared writer lock.
Stale external changes are refused. Invalid startup settings use defaults with a
warning and preserve the original file; RPC startup and permissions remain
available. Failed replacement preserves the last in-memory state. See
[history presentation](history.md) for search/conditions/sort/width persistence.
Export path protection already covers this document.

## Driving it and proof

- Core: `python scripts/cargo.py test --no-default-features --locked`.
  `tests/history_preferences.rs` verifies exact expiration of events/alerts/counters,
  review without mutation, trigger-induced rollback, independent Ping/Allow under
  a held SQLite lock, stale revision rejection, restart persistence, explicit CLI
  overrides in an owned process, gradual-compaction feedback and version-1 view
  migration/merge/invalid values. A real one-minute idle test waits without new
  ingestion and observes expired data disappear; it does not accelerate the clock.
  A schema-7 database lacking the new table migrates with its event intact.
- GPU: `python scripts/verify-headless.py --feature history-preferences`.
  Require ten frames, real GPU and owned cleanup. Actual Kit input rejects zero
  days, reviews/back/applies 1 day/32 MiB/1-minute cleanup, answers a real permission
  while SQLite is locked, rejects refresh 61, saves manual mode, proves continued
  recording without page replacement, Refresh now, saved automatic interval,
  clipboard database path and independent reopen. The automatic check waits for
  the real two-second interval. Review is compact and restores its focus scope.
- Regressions: full flow and `--feature history-tools` exercise existing actions,
  filters, columns, global sort and exports with the migrated document.
- Native: after rebuilding desktop, run
  `python scripts/verify-native.py run --feature history-preferences`.
  Require owned Wayland opening/review/save through keyboard, exact independently
  persisted days=1/MiB=256/cleanup=5 with unchanged fields, 430 history rows, zero
  daemon commands and every cleanup flag. This proves the retention edit path,
  not all native inputs, display settings, file relocation or disk exhaustion.

## Gotchas and bounds

One review/command per window, the existing 32-command writer queue and eight-MiB
telemetry budget; no new unbounded history or worker pool. Settings are scalar,
expiry queries/cleanup run on the SQLite writer, and persistence never holds the
permission snapshot lock. Long storage operations can cause explicitly counted
telemetry skips; permission handling remains independent. Views still retain a
200-row page and bounded filter/column controls. The physical page cap uses actual
page size and reserves one quarter of the requested target for WAL/write overhead.

Database relocation/backend/journal choices, temporary-rule policies, persisted
widths/sorts/filter presets, exhaustive native controls, cap-sized interactive
stress and long-duration RSS remain parity/proof work. The full goal is active.

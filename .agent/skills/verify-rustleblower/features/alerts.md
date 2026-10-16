# Daemon alerts

## User behavior

Open Alerts or Ctrl+5. Search literal text across summaries and complete typed
payloads, optionally enter an exact node name, choose All/Errors/Warnings/Info,
and filter Unread. Latest follows new reports; Older/Previous keep a captured
arrival boundary. Select a row for a virtualized JSON preview; Copy complete JSON
preserves the complete bounded payload, while Copy page as CSV copies summaries.

Mark read updates local archive state. Delete selected and Delete matching both
require confirmation; Cancel changes nothing. Matching deletion captures current
filters and an arrival boundary, preserving alerts that arrive during review.
No action changes daemon rules, counters, interception or firewall configuration.

Enter an absolute export path and choose CSV or JSON. All matches through the
captured arrival ceiling are exported across pages. Existing files stay intact
unless Replace is explicitly selected. Stop is checked before publication;
finished files remain finished. JSON Lines includes typed data and archive
metadata. CSV contains summary/metadata and the complete JSON document.

## Source contract

Installed OpenSnitch 1.7.2 `service.py::_on_new_alert`, `PostAlert`, database
`alerts` schema and `dialogs/stats.py` alert view/delete/export handlers define
the existing behavior. The local wire schema supplies text, process, connection,
application-rule and firewall-rule oneofs plus severity, priority, cause, action
and source ID. IDs are stored as decimal text so u64 is exact.

SHOW_ALERT records and updates the visible latest-alert banner. SAVE_TO_DB records
without replacing that banner. NONE is acknowledged without an action; unknown
actions are acknowledged with explicit skipped-record feedback. Acknowledgement
means RPC receipt, not SQLite durability. Sending-session identity determines the
node, independently of the node selected for browsing. Before subscription or
after session replacement, an explicit unidentified-peer label is used rather than
attributing data to another live node. Unknown severity/priority/cause values are
preserved. Summaries normalize control characters to keep the header stable; the
complete supplied text and typed metadata remain in the archived document.

Desktop notification delivery and per-rule alert settings remain separate parity
requirements; the banner and archive do not prove an OS notification was shown.

## Bounds

One alert is at most 64 KiB encoded / 256 KiB retained input allocations and
512 KiB canonical JSON. Preparation reserves a conservative 4 MiB from the shared
8 MiB ingestion semaphore, then retains only its measured queued allocation.
The existing 32-command writer queue stays bounded and uses a boxed payload.
Rejected/failed alerts have their own skipped/error counters, separate from
connection-history loss. Permission decisions never await alert preparation
queues, SQLite or alert UI visibility.

Schema 7 stores at most 10,000 alerts / 2 MiB logical data and applies configured
history age at startup and maintenance. Oldest records are evicted first. Alert
bytes reserve from the existing conservative history logical budget; SQLite page
limits still apply. The archive includes the payload the daemon explicitly sent,
which can include typed process/connection environment and checksum fields.

The UI retains a 200-summary page and one selected detail, at most 2,048 preview
lines of 512 bytes. Preview shortening is explicit; copy/export keeps the complete
payload. One background read/export permit and 512-KiB read caches bound I/O.
Paging history has a 64-cursor cap. Export captures at most 10,000 records / 4 MiB,
releases the live reader before output, streams through a 16-KiB buffer and caps
output at 16 MiB. Clipboard output is at most 1 MiB. Stop and an existing-file
check precede atomic publication. Database/WAL/SHM/settings paths are protected.

## Verification

- Core: `python scripts/cargo.py test --no-default-features --locked`.
  `tests/alerts.rs` checks two nodes, banner-vs-save-only/NONE/unknown actions,
  all five payload kinds, u64/unknown-enum preservation, exact typed metadata,
  independent reopening, read filtering, JSON/CSV data and existing-file/active
  database protection. A held SQLite write lock plus queue pressure must still
  return Ping and an Allow reply within 500 ms. Stored + skipped equals submitted.
  Model tests reject encoded/retained/depth overflow, preserve complete Unicode
  text with normalized summaries, enforce age/byte eviction, freeze older pages
  during arrivals and preserve new arrivals during matching deletion.
- GPU: `python scripts/verify-headless.py --feature alerts`. Require ten PNGs,
  real GPU, owned child exit and scoped socket removal. Drive Ctrl+5, paging,
  filters, detail/clipboard, read/delete/cancel, full JSON/CSV export, refusal and
  explicit replacement, a real Deny permission and matching deletion during a
  new arrival. Final search must show No matching retained alerts.
  Pace the 206-row paging fixture with a persistence fence after each RPC;
  receipt does not promise durability and a burst may intentionally overflow
  the bounded writer on a slower CI disk. Keep overload/responsiveness proof in
  the separate held-writer core case rather than depending on fixture timing.
- Native: rebuild desktop after core tests; then
  `python scripts/verify-native.py run --feature alerts [--backend x11]`.
  Require Ctrl+5 painted synthetic warning rows, Ctrl+F empty search, six exact
  persisted node/severity/priority/action/text records, 430 connection rows, zero
  policy commands and every cleanup flag. This proves native open/search and
  persistence, not every native detail/export/delete control.

## Ten frames

Latest; older frozen page; error filter; typed process details; unread empty;
selected deletion review; existing-export refusal; independent permission;
matching deletion review; empty search. Exports are retained beside the images.
Native results and current proof boundaries are recorded in the port plan.

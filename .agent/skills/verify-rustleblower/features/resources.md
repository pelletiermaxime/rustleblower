# Bounded operation and storage

Busy traffic and slow storage should leave permission handling responsive while
history remains bounded and reports any loss.

## Sub-features

- `resource-prompts`: pending count and per-request payload are capped.
  Each request is at most 64 KiB encoded / 256 KiB retained; at most 32 are live.
  At most 128 unobserved lasting replies share the catalogue's disk limits.
  Each pending RPC retains one watch deadline, capped at 120 seconds even during
  review. Input does not create tasks or retain event queues; queued requests
  expire independently. Shorter RPC deadlines retain a 100 ms reply margin.
- `resource-ingestion`: telemetry has command-count and memory budgets.
- `resource-icons`: two background lookup/decode workers, 128 cached 64-pixel
  icons/misses for 60 seconds, 2048 desktop entries / 8 MiB per scan and 1 MiB
  encoded icon input. PNG decoding has dimension/allocation limits; SVG image
  references are disabled. The `permission-icons` GPU flow holds both workers
  and requires an immediate permission reply with initials still available.
- `resource-retention`: old history expires and a conservative disk target applies.
- `resource-history-tools`: a shared 200-row page, eight 256-byte AND conditions,
  16-KiB presentation settings (nineteen finite widths, one coalesced snapshot/
  background writer) and background 32-row legacy indexing. The command text
  is at most 8 KiB/64 arguments; metadata allocations share ingestion/retention
  accounting. Export snapshots source files up to 4 GiB, bounds the private copy
  at 8 GiB and streams at most 256 MiB with 512-KiB reader caches and a 16-KiB
  output buffer. Clipboard is at most 4 MiB. Snapshot/temp sort disk is additional
  to the history target; live WAL is not held during output encoding/writes.
- `resource-statistics`: reports use the shared 8-MiB semaphore and one fair latest
  pending slot per node (at most 32), capped at 10,000 keys / 2 MiB including capacities. Disk cache
  keeps at most 32 nodes / 10,000 keys / approximately 2 MiB of counter data,
  reserving from history's quarter-budget before insertion. One background capture
  uses a 512-KiB SQLite cache, at most three 4-MiB private view files across
  current/pending/export, a 200-row UI page and top ten. CSV/JSON output is at most
  16 MiB, clipboard 4 MiB, with a 16-KiB buffer. Separate skip/error/eviction health
  does not inflate history loss.
- `resource-isolation`: disk contention cannot block prompt decisions or Ping.
- `resource-preferences`: reads are capped at 4 KiB; saves run on the background
  executor, serialize independently and hold no prompt-facing lock during I/O.
  RPCs use the in-memory snapshot, and queued requests keep captured defaults.
- `resource-daemon-preferences`: one editor, a 64-KiB object, 29 fixed controls
  and a dirty-field set. Review retains at most 200 rows with bounded displayed
  paths/values; the whole candidate remains subject to the document limit.
  Rebuilding a form drops its subscriptions. Applying uses the existing bounded
  asynchronous node command/ACK/cache path; permissions remain independent.
- `resource-placement`: one cached primary work rectangle and a one-slot latest
  move channel. Monitor/WM reads and acknowledged move requests run on the
  background executor. At most 32 monitor entries are considered, property reads
  are bounded, stale/destroyed windows are discarded and PID ownership is checked.
- `resource-transfers`: one background rule workflow; bounded JSON parsing,
  virtualized review, read-only snapshot cache and sequential mutations.
- `resource-nodes`: 32 current generation gates and 32 cached metadata rows,
  each with 64-KiB JSON / 256-KiB firewall snapshot / 256-byte kernel. Metadata
  payloads and per-node statistics slots share 8-MiB ingestion; node readers share
  one permit and a 512-KiB SQLite cache. Only selected configuration is loaded.
  Node controls share the 16-command cap and ten-second deadlines. Unreachable
  legacy catalogues are pruned while live-loading nodes are preserved; existing
  SQLite page limits still apply to metadata/rule storage.
- `resource-checksums`: one read/prepare worker across probes and one review
  window. Pages retain at most 16 rules / 512 KiB including copied differences;
  the read-only SQLite cache is 512 KiB. UI/RPC paths never wait for its I/O.
  Updates retain one page/cursor, revalidate each rule and cap attempts at 10,000.

## How to get to it (user POV)

- Keep the application connected while traffic is reported.
- Read pending/timeout/overflow counters and any incomplete-history banner.
- Configure `--history-days`, `--history-mib`, and prompt fallback options at launch.

## Driving it with protocol tests and synthetic load

Preconditions: disposable socket/database, no real firewall requests or policy changes.

- **Pressure and contention.** Run `python scripts/cargo.py test --no-default-features --locked`.
  The busy-history test holds SQLite's write lock, fills 32 prompts, checks an
  overflow deny/once, and requires fast Ping/decision replies. Oversized event
  batches increment loss accounting.
  Choice tests also reject a tiny encoded message with oversized retained vector
  capacity, check bounded fallback conditions, and race two choices for one ID.
- **Accurate loss accounting.** The partial-write regression commits the first
  128 rows, fails the next chunk, and checks that only the remaining two records
  count as storage failures. Recovery clears the active error. The shared-budget
  test proves that held startup batches consume the normal budget and release it.
- **Warning rendering.** The background GPU flow captures
  `07-history-warning.png` after an oversized batch, with its specific reason.
- **Retention.** The large-record storage test inserts enough maximum-length
  records to force eviction and checks retained logical bytes and SQLite files.
- **Rule transfers.** The core transfer tests export/import 260 rules, verify
  later pages and a stable snapshot during concurrent writes, preserve existing
  files on failure, reject oversized JSON/counts, and roundtrip OpenSnitch formats.
  The GPU flow answers a separate permission window while holding a bulk reply;
  `25-prompt-during-bulk.png` is that input proof. `26-bulk-all-pages.png` reviews
  229 rules; assertions verify acknowledged commands and later-page persistence.
- **Detailed history.** Snapshots share the normal 8 MiB queue budget and are
  limited to 24 KiB each. Incremental reservation releases partial work on failure.
  Schema/retention tests verify cascading detail deletion, byte accounting,
  remaining recent rows and the existing 16 MiB file-size test target.
- **Exploratory throughput.** Against an owned demo instance, run
  `python scripts/fake-daemon.py --socket /absolute/owned/ui.sock --prompts 0 --burst-batches 100 --batch-size 256 --seconds 10 --report /absolute/report.json`.
  Record app RSS before/during/after, fixture latency, persisted row counts and
  visible loss. This command alone does not establish a performance improvement.

## Gotchas

- Ingestion acknowledgement is not a durability acknowledgement. Overload can
  lose telemetry while prompts remain responsive.
- GPU/driver memory is separate from history budgets; measure total RSS.
- Disk accounting reserves space for indexes/WAL but is not an OS quota.
- Foreground overload-banner rendering and long-duration RSS plateau are not covered
  by the default UI flow. Report those boundaries rather than inferring them.

## Alert archive

Alerts share normal ingestion permits and the writer queue while permission
choices bypass them. Each input is 64 KiB encoded / 256 KiB retained, with a
4-MiB temporary preparation reservation shrunk to queued allocations. The disk
archive is at most 10,000 rows / 2 MiB logical data plus existing SQLite overhead,
and shares age/history-budget maintenance. One 200-row summary page, one selected
bounded detail, one read/export worker and a 4-MiB captured export keep view state
bounded. See the [alert feature](alerts.md) for exact caps and proof boundaries.

## Desktop notices

Eight bounded node/summary events, one pending latest notice, one known server ID
and one pending Open action; a single worker performs all session-bus I/O. Burst
coalescing and a 500-ms interval limit Notify traffic; connection/method timeouts
and cancellation avoid per-notice threads. Service-owned expiry/visibility is not
a memory or delivery guarantee. Alert archival and permission responses have no
dependency on this worker. The desktop-notifications feature specifies exact
bounds, private-bus failure proof and GPU toggle/restoration proof.

## Live inspections

Four daemon read tasks, one latest 1-MiB update each; one dashboard inspection and
one global parse permit. Socket view retention is capped at 10,000 rows / 4 MiB
including raw bytes and measured row allocations. Process indexes are bounded and
paths normalized once per PID. Display pages contain 200 rows. Generic process/
Sysinfo fields retain at most 128 names; preview is at most 2,048 lines × 512 bytes.
Raw JSON copy preserves the admitted complete snapshot. Read tasks never consume
the sixteen policy-confirmation slots or await SQLite. The live feature map
defines stop retry, stale expiry, session-generation checks and exact proof.

# Resource limits

- At most 32 pending prompts, each limited to 64 KiB encoded and 256 KiB of
  retained connection allocations. At most 128 unobserved lasting popup replies
  are stored in the rule catalogue, sharing its rule count and byte limits.
- History ingestion is limited to 32 queued commands and an 8 MiB record budget,
  shared with reports received while the daemon is connecting.
  Each event can retain a detail snapshot of at most 24 KiB. Arguments are limited
  to 64 entries / 8 KiB total / 1 KiB per argument. Environment variables,
  checksums, and process trees are not saved in history.
- The UI holds one history page of at most 200 rows shared by its compact and table views. Both render only visible rows.
- Application rules are stored on disk and queried in pages of 200. Inventories
  are limited to 10,000 rules and 2 MiB of encoded data; an individual rule is
  limited to 64 KiB. At most 16 rule commands can await daemon replies.
- Checksum review holds at most 16 affected rules / 512 KiB per page. One
  background reader serves checks and update preparation, with a 512 KiB SQLite
  cache, no memory mapping and a 250 ms busy timeout. One checksum review window
  can be open; updates send one command at a time and recheck the saved rule first.
- SQLite uses one writer thread, a 2 MiB suggested page cache, disabled database
  memory mapping, and short write batches. Rule exports and import reviews use one
  independent read-only snapshot with another 2 MiB suggested cache. RPCs never
  wait for a database write.
- A rule import/review is limited to 10,000 rules, 2 MiB of encoded rule data,
  and 8 MiB of retained rule allocations. JSON input/output is capped at 8 MiB,
  with a 512 KiB cap per imported JSON object. JSON arrays are read one object at
  a time. Bulk changes send one command at a time; lists remain virtualized.
- Default retention is seven days from receipt, with a 256 MiB disk target.
  `--history-days` and `--history-mib` change these. A conservative logical-data
  budget reserves space for indexes and WAL; the main database also has a page
  limit. SQLite checkpoints its WAL after each write batch. The disk target is
  not a filesystem quota, especially if another process opens the database.
- History warnings report the cause of missed events during the current run.
  Normal connection startup preserves early reports without a warning.
  Ping acknowledgements
  confirm RPC receipt, not durable history storage. Queue saturation can lose
  telemetry; it cannot consume the prompt queue.

For the pinned daemon reference and protocol support, see
[upstream reference](upstream-reference.md). Oversized gRPC messages are rejected
at 2 MiB; unusually large rule inventories may exceed that limit.

Build, architecture and verification procedures are in the
[development guide](development.md).

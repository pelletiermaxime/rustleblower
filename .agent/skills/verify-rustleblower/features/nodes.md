# Nodes and daemon configuration

## User behavior

Open Nodes or Ctrl+4, search a cached name/kernel and select a row. Use node for
application rules chooses that live node's catalogue. Start/Stop interception
requires explicit confirmation and waits for a matching daemon acknowledgement.
Daemon configuration opens the [typed preferences](daemon-preferences.md), with a
Complete JSON Kit multiline editor: edit, Review changes, Back to editing or
Confirm apply. JSON must be an object below
64 KiB; unknown fields remain in the document. Success requires daemon OK and a
saved local cache. Errors preserve last confirmed values and mark only that node
uncertain; refresh/reconnect before retrying node controls. Offline controls stay
disabled, including when cached metadata belongs to another connection generation.

The header names the node selected for rules. Permission popups name their own
node. Changing selected nodes does not cancel another node's prompts/editors.
Refresh node inventory refuses only that node's pending prompts/commands.

## Source contract

Pinned 1.7.2 `daemon/ui/notifications.go` handles ENABLE_INTERCEPTION,
DISABLE_INTERCEPTION and CHANGE_CONFIG. Configuration reload precedes saving;
ERROR may mean memory changed before a disk failure. `daemon/ui/config/config.go`
defines its JSON. Installed `nodes.py`, `dialogs/preferences.py` and `stats.py`
define targeting and node controls. Same-name subscription currently represents
replacement; remote address/peer identity is still pending with TCP/TLS.

## Verification

- Core: `python scripts/cargo.py test --no-default-features --locked`.
  `tests/nodes.rs` uses disposable gRPC connections for independent node routing,
  crossed acknowledgements, per-node reconnect/permission isolation, 32-node
  admission/offline eviction, exact configuration roundtrip and persistent failure
  handling. Same-transport Subscribe must allocate a fresh generation; closing
  the old stream cannot remove the new one. Two simultaneous statistics reports
  must survive per-node FIFO coalescing and Barrier.
- Background: `python scripts/verify-headless.py --feature nodes`. Require eleven
  PNGs, real GPU, socket removed and owned child exited. Actual input selects the
  main tab, row and target rules; cancel/confirm interception must leave the cache
  unchanged until ACK. A beta-node permission is answered while alpha's ACK waits.
  Type invalid JSON then a valid full document, review/back/confirm, inspect the
  actual CHANGE_CONFIG payload and independently reopen its cache. Beta ERROR
  preserves its running state and leaves alpha certain. Ctrl+F reaches empty.
- Native: build desktop after core tests, doctor, then
  `python scripts/verify-native.py run --feature nodes [--backend x11]`.
  Require painted synthetic node/interception metadata, empty-state OCR after
  keyboard search, persisted name/kernel/running/configuration, 430 events, zero
  policy commands and all cleanup flags.

## Eleven frames

Connected nodes; beta's selected rule catalogue; alpha stop confirmation; held
interception ACK; beta's independent permission popup; confirmed alpha state;
invalid nonobject JSON; reviewed configuration; confirmed saved configuration;
beta command ERROR/uncertainty; empty typed node search.

## Bounds and remaining proof

32 registered names/live generations; 32 cached metadata rows; 64-KiB JSON and
256-KiB firewall snapshot per node. Ingestion uses the shared 8-MiB semaphore and
bounded history queue. One node read permit, 512-KiB SQLite cache, summaries only
until selection; one configuration editor. All nodes share 16 pending commands
with ten-second deadlines. Rule pages, proposals and commands retain their normal
bounds. Statistics has at most one fair latest slot per node, capped at 32 with
the shared memory budget. Unique generation checks prevent stale cache updates.

Native interception gestures, resource monitors, TCP/TLS/peer
identity, node export/removal, cross-node bulk copying,
long-duration RSS and real daemon enforcement remain pending. Metadata admission prunes unreachable legacy rule caches while preserving
currently loading live nodes. Test fixtures do not
change the user's real daemon, socket, configuration or firewall. Preserve evidence.

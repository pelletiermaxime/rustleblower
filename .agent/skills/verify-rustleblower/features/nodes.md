# Nodes and daemon configuration

## User behavior

Open Nodes or Ctrl+4, search a cached name/kernel and select a row. Use node for
application rules chooses that live node's catalogue. Start/Stop interception
requires explicit confirmation and waits for a matching daemon acknowledgement.
The opt-in pause preference is described below.
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

Current target a1353848 (source 1.9.0) retains the compatible
`daemon/ui/notifications.go` handles ENABLE_INTERCEPTION,
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
- Background: `python scripts/verify-headless.py --feature nodes`. Require seventeen
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

## Seventeen frames

Connected nodes; beta's selected rule catalogue; alpha stop confirmation; held
interception ACK; beta's independent permission popup; confirmed alpha state;
invalid nonobject JSON; reviewed configuration; confirmed saved configuration;
beta command ERROR/uncertainty; empty typed node search; remembered pause;
reconnect restoration waiting for ACK; independent permission under that restore;
confirmed restored pause; opting out keeps the current pause; manual resume.

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


## Remember a paused node

Select a live node and check **Remember this node's paused state after reconnect
or app restart**. Default is off for every node. Saving is a local SQLite change
and emits no daemon command. It captures the currently confirmed reported state.
Later manual interception changes update the saved choice only after daemon OK,
in the same transaction as the node cache. Cancel/ERROR/timeout preserve the
previous choice. A late SQL error rolls back both cache and preference, explains
that live policy may already have changed, and marks that session uncertain.
Unchecking removes the remembered choice without enabling the daemon.

The reference `config.py`, `dialogs/preferences/settings.py`,
`dialogs/events/dialog.py::_cb_start_clicked` and `service.py::_on_node_actions`
store an opt-in global state and restore a paused single-node installation.
Rustleblower implements the same pause-only restoration with explicit per-node
scope. It never auto-enables a stopped daemon. Names are the existing Unix node
identity; remote transport/peer authentication remains a separate parity item.
Preference memory follows the cache: at most 32 rows, names at most 128 bytes,
4 KiB of name data, and atomic removal when an offline cached node is evicted.
An oversized/unreadable snapshot suspends restoration and shows an error instead
of using partial preferences. Snapshot reads on RPC/UI threads perform no I/O.

On Subscribe, only a daemon reporting running can be eligible. Restoration waits
for that exact generation's saved metadata, ready rule inventory and notification
stream. The existing one-second maintenance task submits a single Disable via
the normal bounded command queue. It obeys node/firewall busy and uncertainty
guards. A manual interception command supersedes an unsent restoration. Once
sent, it is not automatically retried on ERROR/timeout/disconnect. A subsequent
fresh Subscribe may try again. The restoration remains busy through ACK and
SQLite completion; other node configuration/interception commands wait. The Nodes
page observes that state and paints explicit restoration feedback, clearing old
success messages when the selected node or its connection generation changes. The
maintenance task retains at most 32 completion receivers; policy commands retain
the existing 16 pending / 8 outbound bounds. Failures are surfaced in the normal
status feedback. Neither permission replies nor UI visibility gate this work.

`tests/interception.rs` covers opt-in/no unsolicited command, rejected manual
choice, durable Stop, complete backend restart, exact automatic Disable, held
ACK with independent permission, a real SQLite BEGIN IMMEDIATE held after ACK,
continued independent permission and blocked premature resume, successful manual
Start, opt-out with no command, node-scoped routing/crossed ACK, already-paused
reports, never auto-enable, one failed attempt with visible feedback and stale
preference/generation refusal. A late SQL trigger proves both tables roll back.
A unit test drives 33 offline cache admissions, requires preference eviction and
32-row bounds, and rejects a corrupt oversized snapshot without partial restore.
The full core suite now has 140 cases.

The 17-frame GPU flow uses the actual checkbox, confirmation and Cancel controls,
replaces alpha's connection, holds the automatic ACK, answers beta's real AskRule,
then opts out without resuming and explicitly resumes. No state setter drives
product actions. Core proof separately establishes restart with no UI present.

After a desktop rebuild, `python scripts/verify-native.py run --feature interception-preference`
adds owned Wayland keyboard/OCR proof: Ctrl+4, Enter selects the first node, Tab
and Space enable its checkbox. Require two captures, saved-choice feedback, one
exact node_pause row with running=true, unchanged reported interception/config,
zero daemon commands, 430 history rows and all owned cleanup flags. Native
Stop/Start and reconnect gestures remain separate from this preference proof.
The existing native daemon-preferences journey now takes four Tabs from node
search to the configuration button because the checkbox precedes those actions.

# Daemon preferences

## User behavior

Nodes (Ctrl+4) → select a connected node → Daemon configuration. Enter in the
node search selects its first visible row. Settings uses pinned Kit Select,
Input and Checkbox controls in five groups: General, Logging, Firewall & memory,
Statistics and Connection. Twenty-nine fields include policy defaults, process
monitor, rules/checksums, logging, queue/firewall/memory settings, daemon counter
limits, server address and authentication/certificate paths.

Only edited fields change. Missing values retain the daemon's default behavior;
unrecognized values and unknown fields survive an unrelated edit. Complete JSON
edits the entire document. Switching modes preserves the candidate. Review shows
before/after values and JSON paths. Back to editing cancels confirmation without
sending. Ctrl+Enter reviews; Ctrl+Shift+Enter confirms only a staged candidate.
Success requires the correct session's OK and successful local cache update.
ERROR preserves confirmed values and the reviewed candidate, marks that node
uncertain and requires refresh/reconnect before another apply. Closing while an
ACK waits minimizes the editor. Popup defaults remain separate.

Connection settings describe the daemon client. Editing them does not add TCP/TLS
listeners to Rustleblower. Certificate paths belong to the daemon's machine.

## Source contract

Pinned OpenSnitch 1.7.2 `daemon/ui/config/config.go`, `daemon/ui/notifications.go`,
and installed `dialogs/preferences.py`, `auth/__init__.py`, `res/preferences.ui`
define the fields and CHANGE_CONFIG contract. Auth wire values are `simple`,
`tls-simple`, `tls-mutual`; certificate modes use `no-client-cert`, `req-cert`,
`req-any-cert`, `verify-cert`, `req-and-verify-cert`. Reload precedes final
persistence/ACK; ERROR does not prove daemon rollback. Unknown JSON numbers retain
their exact decimal representation through serde_json arbitrary precision.

## Verification

- Core: `python scripts/cargo.py test --no-default-features --locked`. Five
  `nodes::preferences` model tests cover unknown/default preservation, precise
  numbers, removal without damaging absent/scalar parents, field/type/size limits,
  exact Go-duration parsing including i64 nanosecond boundaries, auth spellings,
  escaped paths, Unicode and review limits. Existing node integration tests cover
  held/crossed/stale ACKs, ERROR, SQLite failure and confirmed reopen.
- Kit: `python scripts/verify-headless.py --feature daemon-preferences`. Require
  eleven JSON checkpoints, `real_gpu: false`, child exited and socket removed. Drive every group by
  real input; reject a negative statistics limit without sending. Switch review/
  JSON/settings modes and confirm. Inspect actual CHANGE_CONFIG payload for
  edited fields, preserved unknown exact numbers, and absent untouched fields.
  Hold ACK, require unchanged cache, answer Allow forever through an independent
  permission window, release ACK and independently read persisted configuration.
  A second ERROR must retain the first confirmed document and uncertainty.
- Native: build desktop after core tests, then
  `python scripts/verify-native.py run --feature daemon-preferences [--backend x11]`.
  Drive Ctrl+4, Enter, Tab/Space to open, Return to select Deny, Ctrl+Enter to
  review, Ctrl+Shift+Enter to confirm. Require readable changed-field before/after
  review and saved feedback; independently require exactly one acknowledged
  CHANGE_CONFIG (action 6), only DefaultAction changed, 430 history rows and every
  cleanup flag. The fixture explicitly opts into synthetic config ACKs and writes
  no system configuration. Retain failed runs; OCR of small JSON paths can misread
  glyphs, so assert the human label plus before/after values and exact protocol data.

## Expected observations

General; Logging; invalid Statistics limit; Firewall & memory; Connection; changed
field review; Complete JSON; held ACK; independent permission; saved confirmation;
ERROR with reviewed draft retained. Inspect the review header/footer and row width
as well as input assertions.

The editor has 29 fixed controls, a dirty set and one 64-KiB document. Review keeps
at most 200 rows and 259-byte displayed paths/values, with an explicit total and
Complete JSON for omitted details. Field text is capped at 128/256/1024/4096 bytes
by purpose. Integer bounds match UI/daemon types; positive interval parsing uses
exact integer nanoseconds rather than floating-point rounding. Group scroll state
and form subscriptions are bounded. The existing node command cap and ACK deadline
apply; no disk read/write or telemetry queue is awaited by permission decisions.

Apply-to-all-nodes, every native field/control variant, remote TLS handshakes,
cap-sized interactive stress, other compositors and real-daemon enforcement remain
unproven or pending. No fixture replaces a system socket or modifies the real
daemon/firewall. Use the per-feature evidence reports rather than inferring proof
from build success.

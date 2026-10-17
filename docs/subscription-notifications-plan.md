# Subscription result notifications

Reference: OpenSnitch `a1353848ba1b660320e90cefea782c3fba272c00`,
`ui/opensnitch/plugins/list_subscriptions/list_subscriptions.py:324–339,1025–1068`.
The plugin enables result notices only when `config.notify` is an object. Its
`success.desktop` and `error.desktop` strings override the default subscription
name/status or error text. An empty object enables fallback messages; an absent
or non-object value disables notices. These are independent of daemon PostAlert
messages and missed permission-popup notifications.

Rustleblower implements that configuration in Blocklists → Notifications. Empty
messages use bounded name/status/error text. Disabling saves `notify: null`,
clearing custom messages because the source format has no disabled object state.
The enabled editor preserves other notification keys, nested non-desktop keys,
unrelated plugins and retained source metadata. The normal native/OpenSnitch
backup formats retain this configuration. Native result notices also cover an
explicit manual refresh; that is a deliberate extension beyond the source's
scheduled callback. HTTP 304 counts as a successful check; it does not claim new
data was downloaded.

The editor reads and writes through the existing subscription worker. Its
revision is the source-template revision, shared with import reviews, so a stale
editor or old import review cannot overwrite an intervening source change.
Unknown source configuration remains subject to the existing 2 MiB, depth,
node/key/string and retained-metadata limits. Message inputs are at most 512
UTF-8 bytes with no control characters. Metadata updates are transactional;
budget failure rolls them back. No schema migration or second settings file is
needed.

Only a normal download's committed `updated`, `unchanged` or `error` result can
enqueue a notice. Cancellation, shutdown, removed/stale revisions, and the
independent URL-test path stay quiet. Notification configuration or D-Bus errors
do not fail downloads. A download/publication failure that stops the worker
before it commits a result remains a worker error, not a claimed success notice.

The subscription actor posts to the existing eight-item best-effort mailbox.
One D-Bus worker retains at most one latest pending and one active notice,
coalesces bursts and replaces the active ID. Both kinds honor the global Desktop
notifications preference. Text is bounded and markup escaped. Source title and
native severity wording differ, but successful and failed results have distinct
icons and normal urgency. There are no new per-notice threads, renderer, policy
commands, UI visibility gates or permission-response dependencies.

Each notice carries an explicit action target. Daemon notices open Alerts;
subscription notices open Blocklists, including when the retained dashboard is
hidden. Replacing a notice updates its target even if the service reuses its ID.
Only the current ID and matching action from the actual service owner can open
the dashboard. Dismissed IDs, an old kind's action, unknown actions and another
bus sender are ignored. Notifications never answer permission requests.

Verification must distinguish the pure/configuration and worker tests, actual
private D-Bus protocol, GPUI rendering/input/action routing, and host desktop
banner proof. A green Notify RPC proves service acceptance, not a painted banner
or physical click. All test HTTP servers, buses, sockets and databases are
owned; no system daemon or `/tmp/osui.sock` changes are necessary. The focused configuration/HTTP worker suite passes four cases, and the private
D-Bus test passes on the combined transport build. The nine-frame GPU journey
passes under `.verification/headless/20261017T022211Z-996e1089`, including actual
controls/save/reopen, Back preservation, stale refusal, reload/Ctrl+Enter,
disable-clear, distinct hidden Alerts/Blocklists restoration and a permission
answered by input. Its callbacks are synthetic; the HTTP and D-Bus tests prove
the production outcome/delivery paths separately. Subsequent copy-only changes
remove protocol jargon from the editor; no behavioral test claim depends on that
wording. Host desktop banners and physical notification clicks remain unproved.

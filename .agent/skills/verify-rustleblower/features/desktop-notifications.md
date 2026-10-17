# Desktop notifications

## User contract

SHOW_ALERT emits a desktop notification independently of archival and permission
handling. SAVE_TO_DB and NONE do not notify. The Popup preferences checkbox
"Desktop notifications" stages the change until Save; closing
without saving leaves the active setting unchanged. Legacy preferences default
it on. Disabling notifications leaves permission popups and archive recording
unchanged and closes the worker's current known notice.

The title identifies severity and the body includes the sending node and a
bounded plain-text summary. XML metacharacters are escaped. Low/medium/high
priority maps to freedesktop urgency 0/1/2; unknown values use normal urgency.
The default body action and Open alerts button restore the retained dashboard to
Alerts even when its main surface was closed. They never answer a permission or
change firewall policy. Stale IDs, unknown actions and signals from another bus
client are ignored. The desktop decides whether/where to paint a banner, including
Do Not Disturb behavior; successful Notify means service acceptance, not visibility.

## Architecture and bounds

Installed OpenSnitch 1.7.2 `service.py::_on_new_alert` and `notifications.py::show`
define severity, urgency and opening behavior. The pinned Kit has in-app notices;
the pinned GPUI Linux system API spawns a waiting thread per notice, has no urgency
field and cannot retract by tag. This feature instead uses the already-resolved
zbus 6 through one background worker. No new GPUI backend is introduced.

Eight queued summaries (512 bytes plus a 128-byte node), one latest pending notice,
one known active notification and one pending open action bound retained state.
Bursts coalesce into the latest pending notice and replace the known active ID at
most twice per second. Individual archive records are independent. Queue skips
and errors have separate desktop feedback in Alerts. Two signal queues inherit
the connection's 16-message bound. Connection setup has a two-second timeout;
methods have one second; retries wait two seconds. No desktop I/O runs on GPUI or
the permission executor. Shutdown cancels the worker and best-effort closes its
known ID. An uncertain Notify timeout may leave a server-owned notice without a
known ID; notices request a ten-second lifetime and the desktop controls expiry.
The worker never retries a timed-out notice automatically.

Preferences version 4 adds the boolean and reads versions 1–3 with it enabled.
Old binaries reject version 4. Invalid booleans fail explicitly; the existing
4-KiB atomic document and snapshot isolation remain in effect.

## Proof

- `python scripts/cargo.py test --no-default-features --locked`: bounded queue,
  escaped text/urgency, PostAlert show/save/NONE/muted distinctions, archive while
  muted and preference migration/reopen/invalid-value preservation.
- `python scripts/cargo.py test --lib app::desktop_notifications::tests --locked`:
  launch a private owned `dbus-daemon`, use its explicit address, register a fake
  notification service and exercise actual Notify/CloseNotification/signals.
  Require exact text/hints/actions/lifetime, ID replacement/coalescing, spoofed
  sender/stale ID/unknown action rejection, opening/closure, mute, held reply
  timeout, recovery and cancellation. While Notify waits, real Ping and Allow
  must each complete within 500 ms on an owned gRPC socket. Kill/wait only the
  owned bus; never change the host session service or system daemon.
- `python scripts/verify-headless.py --feature desktop-notifications`: actual
  checkbox/save and independent file reopen, archive while muted, restored Alerts
  after a synthetic notification callback with main window absent, and actual
  permission input plus real RPC reply. Nine screenshots and scoped cleanup, including the subscription editor and Blocklists action.

Private D-Bus protocol proof and GPUI callback/input proof are separate. Neither
proves host compositor banner rendering or a physical notification pointer click.
Permission notification alternatives/actions, other desktop lifecycle notifications
and all upstream per-rule settings remain separate parity work.


## Subscription results

Blocklists → Notifications configures source-compatible `notify` success/error
messages. Empty messages use the subscription name/status or failure text;
non-object source config disables notices. Disabling clears custom messages.
Enabled saves preserve unknown keys; stale source-template revisions refuse a
save, and import/export retain the configuration under existing metadata bounds.
Normal committed manual/automatic downloads post updated/304/error results;
cancelled, stale or removed work and URL probes stay quiet. Manual-result notices
are a native extension of the pinned source's scheduled callback.

The same bounded worker/global toggle handles both kinds. Subscription actions
open Blocklists, Alert actions open Alerts, including hidden-dashboard restore.
The active ID's kind is checked on every signal, including reused IDs. Neither
kind answers prompts. See [implementation contract](../../../../docs/subscription-notifications-plan.md).
Four focused config/worker cases pass with owned HTTP200/304/404, source
preservation/stale refusal and cancelled/stale/probe silence. Private D-Bus
target checks also pass on the combined build; logs are
`.verification/subscription-notifications-core.log` and
`.verification/subscription-notifications-dbus-current.log`. The nine-frame journey passes under
`.verification/headless/20261017T022211Z-996e1089` with actual editor controls,
save/reopen/Back, external revision refusal, reload/Ctrl+Enter, disable-clear,
separate hidden Alerts/Blocklists actions and an input-driven permission reply.
Callbacks are synthetic; HTTP/private-bus tests cover production separately.
A later copy-only cleanup removed protocol jargon without changing behavior.
The later sixteen-frame combined journey below includes that copy.


## Missed permission popups

Popup preferences has a Missed popups tab with a separate notification toggle,
editable template, preview and default reset. The source-compatible default is
`%rule.action% action applied %node.addr%` followed by `Command line: %conn.process_args%`.
The input encodes newlines as `\n`, tabs as `\t` and backslashes as `\\`;
malformed escapes refuse the complete save. Only an unanswered AskRule timeout
posts a notice; the captured fallback rule determines the action, while current
notification preferences determine immediate mute and template. The shared
general toggle and bounded mailbox still apply. The action opens Application
rules and preserves permission handling independently.

Four focused missed-popup cases and four existing preference regressions pass
in `.verification/missed-popups-core2.log`, including actual owned gRPC timeout,
current settings, saturation and UTF-8/template limits. Private D-Bus proof in
`.verification/missed-popups-dbus.log` passes Rules action routing, stale-kind
refusal and same-ID replacement across notice kinds.

The combined `--feature desktop-notifications` journey passed sixteen background
GPU/input frames in `.verification/headless/20261017T024155Z-6f0736eb/result.json`.
Frames 10–16 cover actual default/custom escaped input/preview/save/reopen,
per-type mute, whole-save preservation after invalid escape, reset/re-enable,
and hidden Application rules restoration while a second real AskRule receives
an input-driven Deny once reply. Callback delivery in the GPU journey is
synthetic; core and private-bus tests prove production emission and transport
separately. All PNGs and owned socket/process cleanup pass. Aggregate/source
hashes: `.verification/missed-popups-result.json`. Host desktop banner/panel
clicks and native compositor behavior remain separate proof.

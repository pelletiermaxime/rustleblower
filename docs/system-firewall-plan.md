# System firewall implementation contract

This feature is partially implemented; the full acceptance scope below remains active. Preserve OpenSnitch's
ownership of enforcement. Verification must use synthetic daemon clients and
owned sockets; it must never apply profiles to the user's firewall.

## Source-backed behavior

The initial OpenSnitch 1.7.2 inspection established the contracts below. The
current target is master commit a1353848 (source version 1.9.0); see
[upstream reference](upstream-reference.md). The full-reload handler and staged
policy workaround have also been inspected at that pin.

- `nodes.py::reload_fw` sends `RELOAD_FW_RULES` (5) with the entire protobuf
  `sysFirewall` field. It does not put that configuration in notification `data`.
- `firewall/__init__.py::apply_profile` matches a chain by hook, type, family and
  table, applies its policy and adds profile rules while skipping duplicates.
  A profile's payload is a `FwChain`; predefined `.profile` files wrap it in
  `{"Name": "display name", "Profile": {...}}`. `firewall/profiles.py` loads predefined
  `.profile` files from `/etc/opensnitchd/system-fw.d/profiles/`.
- `firewall/rules.py` supports append, insert, update, delete, duplicate detection
  and rule-field changes. Rule order belongs to its chain; UUID is the stable
  identity. `firewall/chains.py` retains name, table, family, priority, type, hook
  and policy.
- `dialogs/firewall.py::enable_fw` reloads policy changes before changing Enabled.
  Its comment describes a daemon ordering problem: disabling first can prevent
  the policy update. The existing Qt implementation sleeps between these stages.
  Rustleblower needs an asynchronous, explicitly acknowledged staged operation;
  do not copy sleeps onto the UI thread or infer enforcement from local state.
- `dialogs/firewall.py` treats reported firewall Version=0 as unavailable.
  Confirm actual daemon/backend capabilities before offering writes.
- Local `proto/rustleblower.proto` contains SysFirewall/FwChains/FwChain/FwRule and
  the full Statement/StatementValues expression tree. It also declares separate
  ENABLE_FIREWALL/DISABLE_FIREWALL actions (3/4); inspect the pinned daemon handlers
  before choosing between those actions and the UI's full-reload workflow.

The reported wrapper `FwChains.rule` and every chain/rule field must survive an
edit to another field. Position is uint64. Preserve exact integers and expression
ordering through file/clipboard roundtrips. Unknown or unsupported versions must
not be silently rewritten as the pinned format.

## Existing integration points

`nodes::Saved` already persists the reported firewall protobuf in SQLite
`node_info.firewall`, capped at 256 KiB per node and 32 cached nodes globally.
`nodes::Detail` currently exposes configuration but not that firewall blob.
Read, decode, validate, filter and export on background workers using the existing
bounded read permit; do not load every node's firewall into the dashboard.

The shared control path currently sends node command action/data, but does not
populate `Notification.sys_firewall`. Extend it with a typed firewall change,
validated bounded payload and explicit session/revision check. Reuse its pending
command cap, stream backpressure, ACK/error/deadline handling and persistence
boundary. Do not add a second independent policy-command channel.

## Implementation sequence

1. Inspect the pinned daemon reload/enable/disable handlers and profile/rule editor
   sources. Add bounded read/edit models and exact wire roundtrip fixtures for
   complete chains, order, profiles and expression values. Decide how unavailable,
   stale and unsupported configurations are represented before adding writes.
2. Add a node-targeted firewall window using pinned Kit components: status,
   chain/profile navigation, 200-rule pages, search, read/copy/export, then staged
   chain-policy, rule CRUD/duplicate/order/enable edits and complete expression
   editing. File/profile import must review the exact changes before application.
3. Apply an explicitly reviewed full snapshot to its captured live generation.
   Save confirmed local state only after ACK; distinguish acknowledged reload,
   freshly reported configuration and actual enforcement. Preserve failed drafts
   and surface uncertainty after disconnect/timeouts. Sequence policy preparation
   and enable/disable without blocking permission prompts.
4. Verify actual Kit input against a synthetic daemon: all mutation types, exact
   complete protobuf payload, held ACK with independent permission input, rejection,
   cancellation, stale review/reconnect, failed local save and bounded large data.
   Add owned native compositor proof and update the feature map with exact limits.

Completion requires profiles, chains, ordered rules, the complete expression
editor, enable/disable and acknowledged reload workflows. A JSON viewer or a
working enable toggle alone does not complete this feature.

## Current implementation checkpoint

The bounded model/interchange, shared typed RELOAD_FW_RULES command, captured
version/fingerprint/session checks, atomic accepted-cache marker, browser and
complete-document review are implemented. Dedicated chain/rule forms now cover
metadata, create/edit/delete/duplicate/enable/order and structured statement/value
editing with local staging. File interchange and imported/predefined-on-disk
profiles now use the same complete-draft review. Built-in policy presets and
acknowledged enable/disable sequencing are implemented; backend-specific condition
presets/validation and broader native/resource proof remain unfinished.
This is not full parity.

Pinned handler evidence confirms no cases for actions 3/4. Action 5 saves first,
then reports success after two seconds without ErrorsChan feedback, which is
why UI feedback says accepted rather than claiming enforcement. The fixture never
runs a host firewall command. See `.agent/skills/verify-rustleblower/features/firewall.md`
for verification and bounds. Public sources were preserved under ignored
`.verification/upstream/` for this inspection.

Compatibility gate: installed `dialogs/firewall.py::fw_is_incompatible` explicitly
rejects iptables for these GUI workflows. Writes therefore require an explicitly
reported nftables backend and format 1, not just the format number. Unknown or
missing backends remain read only. A successful daemon-configuration command
invalidates the prior firewall fingerprint, because it can replace the backend
or configuration path; refresh inventory before another firewall write.

Field-form verification: thirteen real-GPU frames cover complete staged edits,
near-wire-cap budget refusal, exact full payload, independent permission and
stale-form local recovery. Owned native Wayland proof covers a chain-name change,
keyboard keep/review/apply and durable accepted cache; rule/statement/value native
input remains separate. Corrected ordering preserves Position rather than treating
it as a row index. Pinned system.go reverses the array and skips expression-free
rules; rules.go forwards Position to nftables. Form commands require a statement
before enabling a rule. See `.verification/firewall-forms-result.json` for current
and superseded checkpoints rather than interpreting an earlier passing synthetic
test as proof of actual kernel semantics.

File/profile checkpoint: native Kit path controls and OS prompt entry points,
complete/profile export and clipboard, bounded on-disk profile catalogue,
case-insensitive chain matching and content-or-ID deduplication are implemented.
Removing exact profile IDs deliberately preserves the shown chain policy instead
of guessing a new accept policy. Every mutation stays local until the common
review/apply path. See the feature map for current input evidence and remaining
native-picker/built-in-policy/enable-disable boundaries.

Policy/state checkpoint: prepare active base-chain accept policies using the
captured original rules, wait for daemon ACK and saved cache, then save Disabled.
The global bounded operation lease prevents interleaving same-node commands;
closing the view does not cancel the detached approved sequence. First/second
rejection, cache failure, reconnect and command timeout stop continuation and
preserve earlier accepted state. The current upstream target is the explicit
master pin in `docs/upstream-reference.md`; old table names remain compatible.

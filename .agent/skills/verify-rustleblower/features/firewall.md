# System firewall — partial port

## Current behavior

Nodes → select node → System firewall opens one retained window. It loads only
that node's cached protobuf on the background node-read permit. Browse reported
chains, their policy/type/hook/priority and 200-rule pages, search UUID/description/
target, or open Complete JSON. Cached/offline/unsupported configurations remain
read only. Editing requires an explicitly reported nftables backend. Unknown format versions cannot be rewritten as version 1.

Complete JSON preserves every supported wire field, legacy wrapper rules,
positions as exact uint64 strings, ordered expressions and unknown string values within the supported version.
Review parses on a background worker and shows actual before/after JSON paths.
At most 200 changed fields are listed, with explicit remaining count; individual
review values are shortened at 512 bytes. Back returns to the unchanged draft;
copy retains the full draft. Apply sends the exact reviewed complete configuration.
Enabled changes use an explicitly reviewed sequence: prepare accept policies
while active when needed, then save Disabled; enabling sends the reviewed active
configuration. No automatic profile or firewall change occurs on opening.

An editor captures the session and current firewall fingerprint. Control rejects
a changed fingerprint, a replaced session, a missing/non-nftables backend, an unsupported reported version or
an uncertain node. The shared node-command exclusivity and 16-command cap apply.
No optimistic cache changes occur. A matching successful reply commits the blob
and accepted marker in one SQLite transaction; failure preserves the previous
snapshot and marks node state uncertain. A fresh Subscribe replaces that marker
with reported provenance. Reload cached snapshot reads SQLite, not the daemon;
reopen after reconnect to target its new generation.

## Source contract

Pinned OpenSnitch 1.7.2 `daemon/ui/notifications.go::handleActionReloadFw` receives
RELOAD_FW_RULES=5 with Notification.sysFirewall. It deserializes and saves the
configuration, then waits for ErrorsChan or two seconds without errors before
replying. Success is daemon acceptance, not independently proven enforcement.
The handler does not handle ENABLE_FIREWALL=3 or DISABLE_FIREWALL=4 despite their
enum definitions. The UI reloads Enabled instead. Rustleblower implements that workaround with
acknowledged asynchronous stages instead of fixed sleeps.

`daemon/firewall/config/config.go` defines retained legacy fields and Position's
quoted uint64 representation; `daemon/system-fw.json` reports format Version=1.
Installed UI `firewall/__init__.py::apply_profile`, `firewall/rules.py`,
`firewall/chains.py` and `dialogs/firewall*.py` define the remaining workflows.
The full acceptance scope is in [system firewall plan](../../../../docs/system-firewall-plan.md).

## Bounds and persistence

256-KiB encoded snapshot and 2-MiB decoded owned allocation estimate; 256 groups,
256 chains, 4,096 rules (including wrappers), 8,192 expressions and 16,384 values.
The 1-MiB JSON editor preflights container counts before typed vector allocation;
unknown fields fail explicitly. Paste and edits preserve the last admitted text
on overflow. Read-only parsing and JSON generation run off the UI thread. One global firewall
read job can await the shared node reader; repeated openings cannot accumulate
unbounded waiting jobs. Post-ACK reloading waits asynchronously for that reader. Rule
rows are virtualized; only one firewall editor is retained by Nodes.

Schema 9 adds firewall_accepted, tied to node_info with cascading cleanup. It does
not add a second firewall-policy database. Confirmed cache work uses the existing
bounded history writer and telemetry memory budget. The shared controller holds
only fixed-size fingerprints per live node. Permissions never await this writer,
reload ACK, review parser or window visibility.

## Verification

- `python scripts/cargo.py test --no-default-features --locked`: tests/firewall.rs
  checks complete wire/JSON roundtrip, maximum uint64, legacy fields, ordered
  expressions, atomic move/profile changes, container/byte limits, exact reload
  routing, crossed ACK rejection, no optimistic cache update, Ping/Allow under
  held ACK, stale review/generation rejection, transactional cache rollback and
  unsupported reported versions/backends. Successful daemon configuration changes
  invalidate the old firewall source and require a fresh report. Existing migration tests expect schema 9.
- `python scripts/verify-headless.py --feature firewall`: require eight frames,
  actual Nodes/window opening, 225 rules/paging/search, invalid JSON, keyboard
  Textarea edit, exact before/after, Back without mutation, held ACK with separate
  real permission input, exact complete protobuf and accepted marker, full
  clipboard roundtrip, and rejected-draft/cache preservation. The fixture holds the
  shared reader across ACK, then releases it to prove the post-save view catches up. Input must focus
  the actual Kit Textarea; its accessibility label is not an ElementId selector.
- Core, GPU, desktop build and native proof are separate. The field-form
  checkpoint below adds native chain-name input; enforcement remains unverified.

## Native field editors

Chains and rules now have dedicated Kit input forms. Chain metadata edits retain
all existing rules. Rules expose ID, description, verdict/parameters, unsigned
64-bit position, enabled checkbox, legacy fields and ordered expressions. New and
duplicated rules receive distinct opaque IDs; existing empty legacy IDs survive.
The default upstream sample contains empty IDs, so global validation must not
reject those merely because another field changes. New nonempty IDs cannot
collide with another normal rule. Enabled form commands require at least one
expression: the pinned AddSystemRule ignores an empty expression list. Disabling
a legacy empty rule is allowed; re-enabling it requires a statement.

Statement name/operator and ordered key/value pairs have their own forms. Add,
edit, remove, move and cancel preserve the remaining fields. Lists retain 200-row
pages; one selected statement and one selected value own their input widgets.
Intermediate value/statement keeps check encoded and decoded budgets before
retaining more data. Final Keep in draft validates the complete document on a
background worker and synchronizes the browser and complete JSON.

Remove chain/rule requires a local confirmation. Toggles, order and forms stay
local until the common Review and Apply path. Added/deleted objects are reviewed
field by field with the existing bounded summary. Rule moves change only array order and preserve explicit daemon insertion
positions; new rules default to zero, while duplicates retain source metadata.
The pinned daemon traverses the array in reverse and forwards Position directly
to nftables InsertRule. It is not a row index. No-op moves preserve every field. Cancelling a form changes nothing. A form already open during
reconnect can still keep/copy its local draft; it cannot apply to the new session.
Reload is disabled while editing. Returning from edited JSON parses it before
showing forms, and invalid JSON remains available for correction.

Keyboard scope is SystemFirewall: Alt+E edits the selected chain, Ctrl+N creates
one, Ctrl+Enter keeps the active value/statement/form or reviews the full draft,
Ctrl+Shift+Enter applies only a prepared review, and Escape cancels the current
local step. Nested Ctrl+Enter must never skip unsaved expression/value inputs.

- `python scripts/verify-headless.py --feature firewall-forms`: thirteen frames.
  Require chain edit/create/invalid-name handling, rule creation with maximum
  uint64 and all metadata, enabled checkbox and toggle, cancellation/duplication,
  order and confirmed removal. Require actual expression/value add/edit/delete/
  move/cancel, invalid statement handling, keyboard nested keeps, rejection of an
  additional 8-KiB value against a near-cap synthetic rule, exact full protobuf,
  no earlier command, independent permission under held ACK, accepted cache and
  stale-form local keep/copy without mutation of the replacement session.
- After a desktop rebuild, `python scripts/verify-native.py run --feature firewall-forms`
  proves native Wayland opening, Alt+E chain edit, Ctrl+Enter keep/review and
  Ctrl+Shift+Enter apply. Require four screenshots/OCR, exactly one type-5 command
  with only the expected chain name changed, a 32-byte accepted marker, 430 events
  and all cleanup flags. This is native chain-input proof, not native rule/value/
  expression/picker proof or real enforcement. The Python fixture enables this
  only under `--allow-system-firewall` and never invokes a host firewall.

## Remaining full-parity requirements

Native file/profile/rule/expression/value/picker controls; expression/value paging
input beyond the first page; backend-specific condition presets/validation and
long-duration RSS. A near-wire-cap synthetic snapshot has interactive GPU proof;
that does not establish real kernel validity or memory performance at all caps.
The full system-firewall feature and broader port goal remain incomplete.

## Files and imported profiles

The Files and profiles tab accepts a typed path or native open/save/folder prompt.
Load configuration replaces only the local draft, including Enabled. Review
validates the transition; a disabled draft that still has drop base-chain policies
must first use Prepare disabling in Policies and state. Export full draft writes a
complete canonical document. Export/copy selected chain uses OpenSnitch's FwChain
profile envelope (`Name` and `Profile`) and retains every rule field, expression
order and exact uint64. Bare FwChain JSON is also accepted on import. Outer display
metadata is not firewall policy and is omitted when exporting a selected chain;
the new envelope uses that chain's name.
Every export uses create-new (0600) and never overwrites an existing file.
Cached format-1 drafts can be exported after notification disconnect.

Merge profile matches table/family/type/hook without ASCII case sensitivity,
preserves the destination chain's name/priority and changes its policy. It appends
rules only if neither ID nor complete content ignoring ID matches an existing
rule. Existing legacy empty IDs remain supported. An enabled new profile rule
must have statements. Remove profile rule IDs removes only nonempty exact IDs;
it leaves policy unchanged, including drop. The policy stays explicit in Review.
No hidden accept-policy reset or immediate daemon command is sent.

Profile-folder listing uses `/etc/opensnitchd/system-fw.d/profiles` when the path
is empty, or an explicitly chosen directory. It keeps at most 256 paths, scans at
most 4096 directory entries and displays 200 files per page; it does not retain
every file's decoded rules. Files are regular UTF-8 JSON, at most 1 MiB, with the
same container preflight, 256-KiB wire and 2-MiB decoded limits. All disk, parsing,
merging and serialization work runs off the UI thread. Unknown fields, wrong
chains or overflow retain the prior draft. Enabled changes remain local until
explicit sequence review and Apply. A raw JSON edit is
parsed before switching to the file tab. Back from Review restores the visible
page's focus so Ctrl+Enter can review again.

- `python scripts/cargo.py test --no-default-features --locked --test firewall`
  adds independent exact roundtrip/duplicate/metadata/removal/rollback tests and
  directory, file-size, unknown-field and preallocation-limit checks.
- `python scripts/verify-headless.py --feature firewall-files` drives actual Kit
  path input and buttons, two list pages, merge/repeat/remove, invalid imports,
  full/profile file and clipboard export, existing-file preservation, full draft
  replacement, Review/Back/keyboard review, exact complete gRPC payload, independent
  permission during held ACK, accepted persistence and offline export. Require
  eleven PNGs, successful flow report and owned socket/process cleanup. This
  exercises typed paths; OS dialog interaction and native compositor controls
  remain separate. The policy/state journey below covers the staged transition.

Current checkpoint: `.verification/firewall-files-result.json` is the aggregate
for this slice. It distinguishes the final envelope-compatible eleven-frame
journey and installed-Python interoperability proof from the earlier bare-only
checkpoints. The existing thirteen-frame forms and eight-frame JSON journeys
passed again with the new tab; their production paths were unchanged by the
subsequent isolated profile-envelope parser correction. Old report text saying
other workflows were unfinished describes that journey's coverage, not a current
blanket feature status; future report labels state the coverage boundary directly.

## Built-in policies and reviewed state transitions

Policies and state uses the pinned Kit Checkbox/Button controls. Select a chain
there or in the browser. Input/filter Drop adds missing loopback and
established/related exceptions; output/mangle Drop adds established/related;
forward/mangle Drop changes policy without extra rules. Accept removes only the
corresponding reserved profile IDs. Existing matching IDs keep their current
fields, including enabled state. Selected chain name/table/family/priority and
all other rules/wrappers are retained. This works with old table names and the
current reference's `opensnitch` table rather than hardcoding either.

Enable system firewall changes only the local draft. Disabling prepares accept
policies on **all reported base chains**, including forward; regular chains with
no hook are preserved. This deliberately covers more than Qt's input/output
workaround. Unknown base-chain policies fail atomically for explicit correction.
The common review lists each exact step and at most 200 changed fields overall:
when needed, stage 1 changes only the captured original policies while Enabled
remains true; it never activates newly drafted rules. Stage 2 saves the complete
reviewed disabled draft. If no active drop policy requires preparation, one
reload is sufficient. Enabling uses one complete active reload.

A detached background operation waits for each daemon ACK **and** its durable
cache update before sending the next stage. One global firewall-operation lease
bounds retained plans to two snapshots (at most 4 MiB decoded plan data), blocks
same-node policy/rule/configuration changes and refresh through cache completion,
and releases on success, error, reconnect or deadline. Permissions remain
independent. Replacing a session cannot continue an old lease onto the new one.
Errors retain already accepted stages and the local draft; there is no guessed
rollback or automatic replay. Closing the editor lets the reviewed operation
continue; reopening reads its accepted cache/provenance.

Alt+P opens Policies and state after parsing any edited complete JSON.
Ctrl+Shift+E there toggles the draft enabled state; Ctrl+Enter reviews, and
Ctrl+Shift+Enter applies only an already prepared review. The checkbox also works
through ordinary pointer/keyboard Kit input. Nothing changes merely on opening.

- The focused core suite now has eight tests. New cases prove preset wire facts,
  old/current table compatibility, metadata retention, exact preparation/final
  configurations, global/same-node lease guards, independent real AskRule,
  first/second-stage rejection, first-stage cache failure, reconnect, a real
  command timeout and successful lease release/re-enable.
- `python scripts/verify-headless.py --feature firewall-policy` requires eleven
  frames for actual preset/checkbox/shortcut input, invalid preset preservation,
  no command before common review, exact complete two-stage payloads, no early
  continuation, independent permission under held ACK, closing the editor before
  the preparation ACK and then requiring the second command, reopening/enabling
  and rejected-draft/cache preservation. Frame 08 captures the main window because
  the originating editor is deliberately gone.
- After rebuilding desktop, `python scripts/verify-native.py run --feature firewall-policy`
  drives Alt+P and Ctrl+Shift+E in an owned compositor session, reviews two stages
  and applies them. Require four captures/OCR, exactly two type-5 commands with
  complete preserved fields, active accept preparation then Disabled, a 32-byte
  accepted marker, 430 fixture events and all cleanup flags. This is one native
  disable path, not every preset control or kernel enforcement. Its explicit
  `--system-firewall-drop --allow-system-firewall` fixture only changes memory.

The current reference and new master-parity requirements are recorded in
[upstream reference](../../../../docs/upstream-reference.md). Historical 1.7.2
reports are preserved, while the next priority is the native subscription manager
and then port ranges. System-firewall backend-specific condition helpers and
broader native/RSS proof still remain in the full goal.


## Guided statement and verdict helpers

Expressions → Add guided statement… opens finite Kit family/option/comparison
controls and contextual input. All 17 source families construct ordered wire
values; Rule → Choose guided verdict… validates parameters against the captured
chain hook. Keep changes only the local rule, followed by the existing complete
review/ACK path; Cancel/Escape preserves raw and unknown fields. Bounded local
service-name and interface suggestions supplement numerical ports/ranges/lists
and manual interface names/wildcards. Read
[exact scope and bounds](../../../../docs/firewall-helpers-plan.md).

`python scripts/verify-headless.py --feature firewall-forms` now retains 18 frames.
Frames 15–18 exercise actual dual-protocol/not-equal port input, IPv6 CIDR,
SNAT hook refusal and queue parameters; common assertions require complete wire
payload, held ACK/independent permission and reconnect behavior. The passing run
is `.verification/headless/20261017T022741Z-b0a175e2/result.json`. Preserve the
failed `20261017T022336Z-66b482c8` attempt that found stretched Select hit bounds;
fixed rem-height rows now match their painted triggers. Five dedicated
`tests/firewall_helpers.rs` core cases pass in the final focused run; the original
four are also included in the retained 200-case full-core report. The fifth
requires guided DNAT/redirect hook refusal without moving a captured chain,
while preserving raw-editor reachability. This does not prove native compositor helper input,
all helper option combinations, kernel enforcement or final whole-app visuals.


The expanded current `firewall-forms` journey passes 22 frames under
`.verification/headless/20261017T030013Z-f8b4702d/result.json`. New 19–22 exercise
actual service protocol ambiguity refusal, exact 5553/udp selection, local owned0
interface selection and reload from mutated owned discovery files while keeping
the typed 1234 value. The new selections are checked in the whole expected wire
snapshot before existing held-ACK/permission/stale-reconnect regressions.
Two focused discovery/constructor cases pass in
`.verification/firewall-suggestions-core2.log`. All PNGs and owned process/socket
cleanup pass; scoped hashes/boundaries: `.verification/firewall-suggestions-result.json`.
The test-only RAII hook supplies filesystem paths to production discovery; no
host catalogue, environment or policy is changed. Interfaces remain local to
the UI host. A stale hint in these screenshots receives a copy-only followup.

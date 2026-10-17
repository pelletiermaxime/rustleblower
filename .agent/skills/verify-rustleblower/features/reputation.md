# Optional reputation inspection

Read [source contract and native limits](../../../../docs/reputation-plan.md).
This is an independent GET-only VirusTotal integration. Default disabled,
credentials via an owned 0600 key file, fixed approved HTTPS endpoints, no binary
read/upload, no auto-answer or frozen fallback mutation. Parent prompt effects
require exact session/object capture, current settings revision and untouched UI.

`src/reputation.rs` and its store/worker modules own eight queued lookups plus
one active job, eight separate control commands, three subjects per query,
1–10-second per-subject deadlines, 256-KiB response/16-KiB excerpt caps and
128 cache entries. No-data/excluded/404/401/quota/rate-limit/failure is Unknown,
never a clean report. **Not flagged at the configured threshold** is not a safety
guarantee. The source's new malicious desktop notice and upload ideas are TODOs,
not implemented-source parity requirements.

## Focused worker proof

```sh
python scripts/owned_net.py --evidence /absolute/new/evidence/network.json -- \
  python scripts/cargo.py test --no-default-features --locked --offline --lib reputation::tests
```

The first seven cases pass in `.verification/reputation-20261016/core.log`, with
exact source/log hashes in the adjacent `result.json`: typed input/private range/
exclusion bounds; signed threshold/vote/reputation and safe suggestion semantics;
secret mode/symlink/size/debug/state/serialized-config and stale-file refusal;
GET-only/cache/capture/redacted echo; 401/404/429/error/quota/disabled states;
queue saturation with held-HTTP Reload/stale/cancel/quit; body/timeout limits.
Network fixtures refuse to run outside an owned mapped-user loopback namespace.
No real API key, public API request, host daemon or firewall change is involved.

## Native input contract

The combined settings/report window is an App-global singleton. None may activate
the current view; a captured query may reuse only the same capture, otherwise it
must replace the prior owned inspection. A manual query gets a new capture and
cannot later be mistaken for an older popup report. The independent view/ticket
may remain open after the original popup is gone; replacing/closing it cancels
its owned work. Closed handles must reopen cleanly.

Controls are `rep-tab-settings`, `rep-tab-results`, `rep-enabled`, `rep-key-file`,
`rep-timeout`, `rep-budget`, `rep-threshold`, `rep-domains`, `rep-ips`, `rep-hashes`,
`rep-suspicious`, `rep-community`, `rep-reputation`, `rep-recommend`,
`rep-excluded-domains-add`, `rep-excluded-domains-N`,
`rep-excluded-domains-remove-N` (and equivalent `ips` controls),
`rep-save`, `rep-reload`, `rep-query`,
`rep-check`, `rep-captured-check`, `rep-cancel`, `rep-close`, `rep-scroll` and
indexed `rep-official-N`, `rep-details-N`, `rep-copy-N`. Exclusions use up to 32
validated Kit Input rows per category, 128 bytes per row. Pinned TextareaState
does not expose the validation API, so these bounded fields use Input rows.
Inputs never contain key contents.

The integrated singleton view/backend/prompt code passes a desktop check in
`.verification/reputation-20261016/desktop-check.log` (3.57 seconds); strict
no-default-features/all-target Clippy passes in `core-clippy.log` (3.76 seconds).
The initial failed integration compile is retained in
`.verification/reputation-integration-check.log`; it identified unsupported Kit
Textarea validation/id and width helpers. Rendered/input proof remains separate.

Use test-only `Worker::start_fixture(path, base)` and a dummy owned key file.
Require actual Kit input for staged/save/reopen, invalid and stale refusal,
captured/manual scopes, per-subject Unknown/warning/partial states, official
links and plain-text excerpt. Hold HTTP while completing real AskRule/Ping;
then cancel/reload/change capture and prove old data/recommendation cannot apply.
Verify singleton reuse/replacement and popup destruction independently of
background worker tests. A desktop compile or core pass does not establish this
rendered/input/parent-integration boundary or a live-API/package claim.

## Actual GPU/input proof

`python scripts/verify-headless.py --feature reputation` passed 15 frames in
`.verification/headless/20261017T031615Z-c9807eec/result.json` (41.66-second test,
59.147-second wrapper). `.verification/reputation-result.json` links all PNG
hashes, the full 118-file precompile snapshot and the current combined 238-case
core run. The focused seven worker cases are included in that combined count.

Actual Kit input proves default disabled/no HTTP, invalid and stale Save refusal,
save/reopen, bounded exclusion rows, owned GET-only warning/error reports,
redacted excerpt/copy and cancellation. Real AskRule/Deny completes while HTTP
is held. Untouched warnings emphasize Reject once; a warning delivered after
5-minute review preserves the user's duration/Allow decision. Captured inspection
reuse/replacement, old-result isolation and immediate disable clearing also pass.
Only a dummy 0600 key and fixed owned loopback endpoint are used. Official report
links, public API requests, upload, real credentials, native compositor and
packaging are outside this proof.

Visual inspection of frames 3/5/10/11/14 found readable settings, reports and
actual decision buttons. Collapsed frame 10 partly clips its final advanced-details
row; this is recorded as remaining layout polish. The expanded late-warning
frame retains readable controls. Subsequent localization/formula changes need
separate shared regressions; this immutable proof does not imply later-source
identity.

A later complete rerun closes the prompt layout finding: fifteen frames pass in
`.verification/headless/20261017T034508Z-037b159a/result.json` (50.76 seconds).
The corrected prompt keeps preview, decisions and More in a fixed footer while
content scrolls. Three actual wheel events assert exact unchanged footer bounds
and entire Reject once/More controls inside the viewport; captured report buttons
are revealed with actual wheel input. A button touching the exact scroll edge
is fully visible, so the helper adds no arbitrary inset. Original settings,
redaction, held-permission and capture/isolation assertions all pass again.
`.verification/reputation-layout-result.json` retains the full coherent source,
images, scroll metric artifact and failed helper attempts. Frames 10/14 now show
complete readable footer/report controls; the original clipping paragraph above
is historical to its earlier snapshot. Native compositor/package/live API proof
remains separate.

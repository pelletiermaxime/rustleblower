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

Use test-only `Worker::start_fixture(path, base)` and a dummy owned key file.
Require actual Kit input for staged/save/reopen, invalid and stale refusal,
captured/manual scopes, per-subject Unknown/warning/partial states, official
links and plain-text excerpt. Hold HTTP while completing real AskRule/Ping;
then cancel/reload/change capture and prove old data/recommendation cannot apply.
Verify singleton reuse/replacement and popup destruction independently of
background worker tests. A desktop compile or core pass does not establish this
rendered/input/parent-integration boundary or a live-API/package claim.

## Headless verification

Run `python scripts/verify-headless.py --feature reputation`. Require default disabled/no HTTP, invalid and stale Save refusal,
save/reopen, bounded exclusion rows, owned GET-only warning/error reports,
redacted excerpt/copy and cancellation. Real AskRule/Deny completes while HTTP
is held. Untouched warnings emphasize Reject once; a warning delivered after
5-minute review preserves the user's duration/Allow decision. Captured inspection
reuse/replacement, old-result isolation and immediate disable clearing are required.
Only a dummy 0600 key and fixed owned loopback endpoint are used. Official report
links, public API requests, upload, real credentials, native compositor and
packaging are outside this proof.

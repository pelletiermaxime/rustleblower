# Raw file downloads

The optional OpenSnitch downloader action saves raw URL bytes to explicitly configured local files. It is independent of normalized blocklists and daemon Live tasks. Source configure UI is empty; Rustleblower provides a native singleton profile/review window as a native improvement.

Read [implementation/bounds/source contract](../../../../docs/raw-downloads-plan.md). Automatic downloads default off, imported source profiles stage off, manual runs require a saved revision and exact destination review. Worker-based profile I/O avoids filesystem work on the UI thread. Unknown source plugin metadata survives interchange without executing other actions.

Eight isolated focused cases pass: `python scripts/owned_net.py --evidence <owned-path> -- python scripts/cargo.py test --no-default-features --locked --offline --lib raw_downloads::tests`. Retained proof is `.verification/raw-downloads-20261016/hardened-core.log` and `network-hardening.json`. Binary GET/error/size/cancel/changed-file/first-interval behavior and owned-stage recovery are covered. Native window integration, actual input and notice routing are separately proven below; do not infer them from these worker tests.

## Native input and action proof

`python scripts/verify-headless.py --feature raw-downloads` passed twelve actual
GPU/input frames in `.verification/headless/20261017T033900Z-0cf21895/result.json`
(35.56-second test; 49.291-second wrapper). Full 129-file rendered source snapshot,
all image hashes, retained 33-byte NUL/0xff binary and unknown-preserving exported
profile are linked by `.verification/raw-downloads-result.json`.

The journey stages imported settings OFF without writes/HTTP, reviews exact
URL/destinations, cancels review, refuses interval 0, saves 300 Minutes and reopens
without immediate scheduling. Actual reviewed runs publish exact binary bytes;
HTTP 500 and cancelled held GET preserve last-good content. Real AskRule/Deny
completes while HTTP is held. Actual backend mailbox success/error notices arrive;
cancel is quiet. A frozen RunNow revision refuses external settings changes
without HTTP or file mutation. Export preserves unknown plugin metadata and
refuses an existing file. Synthetic RawDownloads action opens/reuses its owned
dialog while retaining the hidden dashboard, following Updates semantics.

The separate private D-Bus regression passes in
`.verification/raw-downloads-notifications-dbus.log` (7.08 seconds), including exact
RawDownloads body/urgency/action and wrong-kind action refusal; eight backend/
settings integration cases pass in `raw-downloads-integration-core.log`. Final
worker eight source includes later stage-parent fsync and highlighting-path guards;
those deltas are core-only relative to the GPU snapshot. Native compositor,
host banner delivery, package and real external download/daemon proof remain
outside this owned namespace run.

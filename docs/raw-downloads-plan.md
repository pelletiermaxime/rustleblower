# Raw URL-to-file downloader

The pinned OpenSnitch source is `a1353848ba1b660320e90cefea782c3fba272c00` (1.9.0). `ui/opensnitch/plugins/downloader/downloader.py:23–104,123–217` defines a disabled global action containing named interval/unit groups, individually enabled URLs with local filenames, and group success/error desktop messages. `utils/__init__.py:116–139` waits the first interval before running. The example `plugins/downloader/example/downloaders.json` supplies the interchange shape. `configure()` and `_gui.py` are empty/TODO; a complex upstream configuration dialog is not a parity gate.

Rustleblower implements this action independently from normalized blocklist subscriptions and daemon Live downloader tasks. It streams raw binary bytes without hosts parsing or policy changes. Native adaptations use one worker instead of a thread per URL, correct a day to 86,400 seconds (the source multiplies by 216,000), and publish a synced same-directory temporary file instead of truncating the current destination before HTTP completion.

## Saved model and review

`raw_downloads::Worker::start(database, socket, callback)` owns configuration, profile I/O, HTTP and publication. `Handle::snapshot()` is memory-only. The command queue has eight slots and refuses overflow. Commands include Save/Reload, bounded JSON Import/Export, ReadProfile/WriteProfile, revision-captured RunNow and Cancel. Save/Import require the captured optional SHA-256 revision. ReadProfile stages a source document with automatic downloads off; unrelated source actions and unknown outer/downloader/group/resource/notification fields survive export as metadata and are never executed. Native configuration is appended to the full database filename as `.raw-downloads.json`; the recovery record uses `.raw-downloads-journal.json`.

The native singleton window is implemented in `src/app/raw_downloads_view.rs`: a validated filename input stages or exports a profile, staged automatic enable and interval/unit controls configure groups, and a review lists exact enabled URLs and file destinations before saving or manually running. Manual Run uses saved settings even when automatic scheduling is off and checks the reviewed revision against memory and disk before starting. Export uses a new filename and never replaces an existing file. Closed windows can be reopened; one App-global handle prevents accumulating owned windows. Application lifecycle and bounded notice/action routing are integrated. GPU and private-D-Bus proof are recorded separately from the core worker proof below.

## Bounds and file ownership

The configuration/profile limit is 64 KiB: at most 16 groups, 64 total resources, names 128 bytes, URL 2,048 bytes, absolute destination path 4,096 bytes and positive finite intervals from one second through 365 days. URLs allow HTTP(S), refuse credentials and fragments, and are not included in error logs/notices. Redirects are limited to three, refuse credentials and HTTPS downgrade. Group desktop messages are at most 1,024 bytes; aggregate failure names retain at most eight plus the total failure count.

There is one HTTP request/file writer at a time, a five-second connect timeout and 30-second overall request/stream deadline, one idle pooled connection and no overlapping or catch-up runs. Scheduling waits the saved interval at startup and after each completed run. Cancellation/settings changes drop an active HTTP future and owned stage. Runtime shutdown waits at most 250 ms for its blocking pool; this does not claim that the OS interrupts DNS or a filesystem sync.

Each resource is at most 16 MiB and an attempted group batch at most 128 MiB; failed streamed bytes consume the same batch budget. With 64 configured destinations the configured current output set is at most 1 GiB plus one 16 MiB stage. Historical files at destinations the user later removes from configuration remain user files; this is not a global disk-retention guarantee. The transport, TLS, allocator and filesystem have additional memory, and no RSS/attack-latency measurement is claimed here.

Destination validation rejects symlinks/special files, duplicate normalized paths and existing inode aliases, the active socket and `/tmp/osui.sock`, database/WAL/SHM, Rustleblower preference documents, the normalized subscriptions directory and private raw-stage filenames. Existing regular destination mode is retained. Existing content fingerprints are checked before publish; first creation uses `persist_noclobber`. A changed destination preserves the external file. The native action never writes daemon configuration or policy.

A private durable journal records the owned stage path, device/inode and UID before downloading a body. Reopen/reload cleans only the matching regular owned stage. A foreign replacement is preserved and recovery is refused. A completed rename followed by directory-sync/journal-cleanup failure is reported as recovery needed rather than falsely promising unchanged output. There is a narrow empty-stage-creation-to-journal gap, and fingerprint checking plus rename is not a universal atomic compare-and-swap against a hostile external filesystem writer. Parent directory replacement races are not claimed solved by pathname validation.

## Current proof

Eight focused tests passed with isolated mapped-user, loopback-only HTTP fixtures: source metadata/default-off roundtrip; protected aliases/stale/corrupt configuration; binary atomic publish and owned-stage crash recovery including foreign-inode preservation; profile export no-clobber/size limits; actual binary GET, HTTP/truncated/oversize last-good preservation and notices; held cancellation/external destination change; first-interval and nonoverlapping scheduling. A stale reviewed run produces no HTTP request. Final hardened log: `.verification/raw-downloads-20261016/hardened-core.log` (compile 3.00 s, runtime 1.32 s), namespace evidence `network-hardening.json`. Live-stage and journal replacement are also preserved during cancellation/publication refusal. Strict core all-target Clippy passed in 3.72 s (`core-clippy.log`). An earlier integer type compile failure remains in `initial-core.log`; the six-case intermediate pass is retained in `core.log`.

These are core, filesystem and owned HTTP proofs. They do not prove native input, desktop delivery, real daemon compatibility, package startup or real external downloads. No actual API credentials, Internet fetches or system daemon/socket changes were used.

Root application integration desktop check passed separately in 4.38 s (`.verification/raw-downloads-integration-check.log`) before the store-only foreign-file cleanup hardening. The subsequent12-frame GPU journey and private-D-Bus action routing now pass; see the feature map and `.verification/raw-downloads-result.json` for exact immutable binary/source scope.

Concrete headless-feature check passed in 4.63 s after the observed-host cfg shadow repair (`headless-check.log`); strict headless library Clippy passed in 4.15 s after two view style corrections (`headless-clippy-final.log`). Worker core hashes were unchanged by these view-only changes.

Final narrow post-GPU safety refinements passed all eight cases again (`final-protected-core.log`, namespace `network-final-protected.json`): Cancel syncs the stage parent before clearing its recovery journal, and the new highlighting preference path is protected. The raw12 GPU snapshot predates those two deltas; the aggregate must keep that boundary explicit. `.verification/raw-downloads-20261016/result.json` records final worker hashes.

Native input now passes the twelve-frame real-GPU journey
`.verification/headless/20261017T033900Z-0cf21895/result.json` (35.56 seconds).
It proves actual profile import forced OFF, exact URL/path review/cancel, invalid
interval refusal, 300 Minutes save/reopen/no immediate dispatch, unknown-preserving
saved export/no-clobber, exact 33-byte binary publication, HTTP error/cancel
last-good preservation, actual permission response during held GET, frozen
RunNow revision refusal, actual worker mailbox notices and synthetic hidden
dashboard dialog/singleton action routing. The dashboard remains hidden when
the independent raw dialog opens, as with Updates. Images 4/7/12 were visually
inspected: exact review paths and actual controls are readable without overlap.
`.verification/raw-downloads-result.json` retains the full 129-file source snapshot,
image/artifact hashes and all boundaries. Separate real private-D-Bus action proof
passes in 7.08 seconds; eight backend/settings integration cases also pass. Later
stage-parent fsync/highlighting guards retain their separate worker eight proof; native
compositor, host notification banner, package and external downloads remain
unclaimed.

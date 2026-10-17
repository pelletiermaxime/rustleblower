# Release checking

Read [source contract and native limits](../../../../docs/update-checker-plan.md).
This ports the configured schedule/notice intent of the pinned OpenSnitch
versionchecker plugin. It never downloads or installs software.

The worker is disabled for automatic network checks by default. Explicit manual
Check now uses saved target/prerelease settings and is itself network opt-in.
The targets are the compiled Rustleblower version and the selected daemon's
actual reported version. Unknown versions are uncomparable. Stable GitHub 404
is an honest no-stable-release result, distinct from current or available.

`src/updates.rs` owns one independent thread, one HTTP future, eight commands,
a latest daemon watch, fixed official endpoints, bounded redirects/deadlines,
256-KiB responses/thirty entries and a 4-KiB revision-guarded preference file.
The full database filename receives `.updates.json`; it must be protected by
native export guards. Corrupt/stale/symlink/special files are preserved.

## Focused core proof

```sh
python scripts/owned_net.py --evidence /absolute/new/evidence/network.json -- \
  python scripts/cargo.py test --no-default-features --locked --offline --lib updates::tests
```

The initial eight-case checkpoint passes in
`.verification/update-checker-20261016/core.log`; the final nine-case rerun in
`final-core.log` additionally covers held-HTTP shutdown and competing first-file
creation after the lazy-client/bounded-runtime refinements. The cases cover
default disabled/no request,
manual opt-in, saved target/prerelease scope, semantic precedence/build metadata,
deduplicated official-link notice, stable 404, rate limit/body/release-count
limits, unknown daemon without HTTP, cancel and changed-context discard,
identical metadata retention, startup/interval/no-overlap scheduling, ten-second
timeout, invalid/stale/symlink/special settings and revision payload refusal.
Network cases require the owned mapped-user namespace and never call GitHub.
Final source/log hashes are retained in the adjacent `result.json`. Strict
core Clippy currently reports unrelated history/i18n warnings in
`core-clippy.log`; a current whole-tree lint result remains separate.

## Native verification contract

Application lifecycle, selected-node context and shared notification/action
routing are separate root-owned integration points. The native window uses
`updates-enabled`, `updates-on-start`, `updates-interval`, `updates-unit`,
`updates-target`, `updates-prereleases`, `updates-save`, `updates-reload`,
`updates-check`, `updates-cancel`, `updates-release`, `updates-close` and
`updates-scroll`. Require real Kit input to save/reopen preferences, reject an
invalid interval and stale revision, distinguish saved settings from staged
manual-check scope, cancel a held owned HTTP request, and open the Updates
action while the main window is hidden. Prove AskRule/Ping while HTTP is held.
Use private fixtures and bounded notification mailboxes. Build/core results do
not establish rendered/input, desktop delivery, packaged or live-API proof.


The focused `--feature updates` journey passed eleven background GPU/input frames
in `.verification/headless/20261017T025610Z-8365eb0f/result.json`. Actual Kit
checkboxes and nonsearchable Select keyboard input save/reopen preferences,
reject zero interval and stale revisions, and restore external saved settings.
The owned loopback fixture proves default zero traffic, unknown daemon zero
traffic, manual Available/Current/rate-limited results while automatic checks
are off, cancel and a real permission reply while HTTP is held. Reopening keeps
Available; a synthetic hidden Updates action preserves a held check and reuses
the single window. All PNGs, network isolation and owned socket/process cleanup
pass. The aggregate `.verification/updates-result.json` links nine focused core
cases, four root integration cases, private D-Bus proof and exact scoped source
hashes. Earlier failed harness attempts remain retained. Official API, release
page browser opening, native compositor, banner delivery and installation are
separate proof boundaries.

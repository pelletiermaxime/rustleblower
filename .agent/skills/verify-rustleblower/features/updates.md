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

Run `python scripts/verify-headless.py --feature updates` for settings input,
save/reopen/stale refusal, opt-in traffic, manual results, cancellation, singleton
reuse and permission input during a held loopback HTTP request.

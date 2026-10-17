# Opt-in release checking

The authoritative source is OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`,
`ui/opensnitch/plugins/versionchecker/versionchecker.py:36-49,76-103,124-159`.
Its plugin is disabled by default, supports checking on startup and a configured
interval/unit, and sends an available-version notification. Its `configure()`
body is empty. The native implementation provides a usable settings window
without copying the source's inconsistent string comparison, hardcoded URL
override, or incorrect days-to-seconds arithmetic at lines 176-200/93.

`src/updates.rs` owns a dedicated background thread/runtime. `src/app/updates_view.rs`
uses the pinned Kit Checkbox, Select, Input and Button controls, with staged
preferences and explicit Save/Reload/Close. The footer stays outside the scroll
area; Select triggers have bounded rows. I/O and permission/telemetry admission
have no shared queue or lock. Parent-owned application/backend hooks supply the
selected daemon version and dispatch a bounded notification callback.

Automatic checking is disabled by default. The initial schedule is twelve hours,
with checking on application startup enabled only when automatic checks are
enabled. The whole interval must be 1 second through 365 days, in integer
seconds/minutes/hours/days. A completed check starts the next interval; missed
ticks do not accumulate and checks do not overlap. Rate-limited automatic checks
wait at least sixty seconds. Saving/reloading schedules the next interval rather
than inventing a new application startup. A manual **Check now** is explicit
network opt-in even with automatic checking off, and always uses saved target
and prerelease settings. Unsaved settings never silently change its scope.

The target is either Rustleblower's actual compiled package version or the
currently selected daemon's bounded reported version/node. Unknown daemon
versions are uncomparable; no installed package version is guessed. Repeated
identical daemon metadata does not cancel a check. A changed selected daemon,
saved settings, Reload, Cancel or shutdown discards the old HTTP future and its
potential notification.

Only the official GitHub API endpoints for `pelletiermaxime/rustleblower` and
`evilsocket/opensnitch` are supported. Stable checks use `/releases/latest`; 404
means no published stable release, including repositories containing only
prereleases. Explicit prerelease mode reads `/releases?per_page=30`, ignores
drafts, and selects the highest comparable semantic version among that bounded
page. It does not claim to search all historical releases. Build metadata does
not make an equal semantic version newer. API-provided release links are ignored;
the link is constructed from the official repository and validated semantic tag.
Checks do not download or install software, change the daemon, or update the UI
package automatically. Custom API URLs, arbitrary actions and credentials are
outside this native slice.

Resources are explicit: one worker, one in-flight request, eight queued commands,
one latest daemon snapshot, 10-second total/5-second connection deadlines, at
most three HTTPS redirects within `api.github.com`, 256 KiB of decompressed
response data, at most thirty release entries, 128-byte node/version/tag bounds,
and a 4-KiB versioned settings document. These are payload/concurrency bounds,
not measured RSS or adversarial latency proof. Notification deduplication keeps
one last target/current/latest tuple; callbacks must use a nonblocking bounded
mailbox. Cancellation drops the HTTP future; it does not retain abandoned jobs.
The HTTP client is created lazily for the first eligible check and keeps at most
one idle connection per host. The runtime allows at most two blocking threads
and waits at most 250 ms for them during shutdown. An operating-system DNS lookup
already running may finish after the update worker exits; cancellation does not
claim to interrupt that OS call.

The preferences path appends `.updates.json` to the full database filename, for
example `history.sqlite3.updates.json`; distinct database extensions cannot
collide. Reads reject symlinks and special files and stop after 4 KiB. Save checks
the captured SHA256 revision, writes a synced owned temporary file, checks the
revision again, then publishes atomically and syncs the directory. First creation
uses `persist_noclobber`. Existing-file replacement still has a narrow external
writer check/rename race, as other native preference documents do; no universal
filesystem compare-and-swap is claimed. Invalid/stale/unknown documents are
preserved and automatic checking falls back to disabled. Repair of an invalid
document requires explicit external correction/removal and Reload. Parent-owned
export guards must protect the final preferences path.

The worker/API model, native settings view and root-owned application lifecycle,
selected-daemon watch, bounded notification/action routing and export guard are
implemented; desktop integration compilation passed before the final
view-opening/bounded-shutdown refinements. The final focused nine-case worker
rerun passes, including held-HTTP shutdown and competing first-file creation.
Exact final source/log hashes are in
`.verification/update-checker-20261016/result.json`; the initial eight-case
checkpoint is preserved separately. A strict core Clippy attempt currently
reports unrelated history/i18n warnings, retained in `core-clippy.log`.
Rendered/input proof is a
separate checkpoint. Focused tests use private
loopback fixtures inside `scripts/owned_net.py`, never the public API or a host
daemon. Preserve `.verification/update-checker-20261016/` for exact logs, source
hashes and namespace metadata. No production outbound update request has been
made during this implementation. A test-only `Worker::start_fixture` hook is
available to the headless harness; it checks mapped-user/loopback isolation and
refuses non-loopback/custom-path/credential-bearing origins. Production builds
retain only the fixed official endpoints.

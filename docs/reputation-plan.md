# Optional VirusTotal reputation

The source contract is pinned OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`:

- `plugins/virustotal/virustotal.py:147-225,294-414,437-555`: configured GET
  endpoints/key/timeout, domain/IP/reported-hash checks, exclusions, analysis
  counts, threshold, suspicious results, signed community votes and reputation,
  popup warning and Reject default-action effect.
- `_models.py:147-240`: hosts analysis, local-range skipping, cached results and
  a per-runtime API-query budget. Its counter is not a documented daily quota.
- `_procdialog.py:35-68`: on-demand existing file-hash report and unknown hash.
- `_popups.py` and `_utils.py`: result details and official report links.

The implementation is independently written Rust. It uses the functional
contract; no Python/GPL implementation or HTML-rendering code is copied.

`src/reputation.rs` owns bounded settings, subjects, captured identity and typed
results; `reputation/store.rs` owns settings and secret-file I/O;
`reputation/worker.rs` owns independent HTTP/cache/control. The native Kit view is
`src/app/reputation_view.rs`, opened with a worker handle and optional immutable
captured query. Parent-owned backend/app/prompt hooks are separate integration
points. The module has no prompt/policy handle or permission-reply method.

The feature defaults disabled. Enabling it explicitly permits sending chosen
domain/IP/reported MD5/SHA1/SHA256 values to VirusTotal. It never opens a binary,
computes a new executable hash, uploads file contents, creates firewall rules or
installs a policy. Official fixed endpoints are
`https://www.virustotal.com/api/v3/domains/`, `ip_addresses/` and `files/`;
redirects/custom endpoints are refused. Opening a constructed official report
link is a separate explicit browser action. No API key or real Internet request
is used by verification.

The configuration stores only an absolute API-key **file path**, not key text.
The path is bounded to 4096 bytes with no controls. The worker opens the final
file with `O_NOFOLLOW|O_NONBLOCK`, requires a regular file owned by the current
effective user with permissions 0600, and reads at most 256 visible ASCII key
bytes plus an optional LF/CRLF. Debug output redacts the secret wrapper and the
HTTP header is sensitive. Response excerpts replace an exact reflected key with
`*`; errors never interpolate request/header/file contents. The temporary secret
buffer is cleared on drop on a best-effort basis; no claim is made that every
HTTP-stack copy is securely erased from process memory.

Settings append `.reputation.json` to the complete database filename. The
versioned document is at most 16 KiB. Eight settings commands are separate from
eight queued lookup jobs, so Save/Reload cannot sit behind a held network body.
Revisions are None or 64 hexadecimal SHA256 bytes, validated before queuing.
Saves capture that revision, recheck after syncing an owned temporary file, use
no-clobber first creation and atomically replace a confirmed existing file.
Corrupt/stale/symlink/special documents are preserved. A narrow external-writer
fingerprint/rename race remains for existing-file replacement; this is not a
filesystem compare-and-swap claim. Export guards must protect this file.

One worker thread/current-thread runtime executes at most one HTTP lookup. A
query has at most three subjects, each domain at most 253 ASCII bytes, IP typed,
and hash 32/40/64 hexadecimal characters. One active query plus eight queued
queries retains only small subject/capture values, never full connections.
Timeout is 1–10 seconds per subject, default one. There are at most two runtime
blocking threads, one idle HTTP connection per host and a 250-ms shutdown wait;
an already running OS DNS call may finish later. The HTTP client is lazy. The
decompressed response cap is 256 KiB, retained plain report excerpt 16 KiB,
cache at most 128 entries with a 24-hour TTL and FIFO eviction. These are explicit
payload/concurrency limits, not measured RSS or adversarial latency guarantees.

The dispatch budget defaults to 500 and accepts 1–5000 actual network dispatches
per worker lifetime. Cached results do not consume it. Reload/Save do not reset
spent requests. HTTP 204/403/429 produce an explicit request-limit state and a
five-minute cooldown, 401 invalid-key, 404 unknown/not-found, and timeout/malformed/
oversize/other HTTP failure Unknown. None is interpreted as a clean report.
Per-subject statuses remain visible even when another subject succeeded.

Automatic connection checks skip local/private/link-local/multicast destinations,
the VirusTotal host itself, and up to 32 configured IP/domain fragments of at
most 128 bytes each. IP fragments retain substring matching; domain fragments
match normalized lowercase DNS names. IPv6/mapped IPv4 checks use typed address
semantics instead of the source's incomplete regex. Whole automatic-connection
exclusion also skips its hash; a separately requested process-hash inspection
does not need a destination. Category toggles still apply to manual queries.

The warning score is `malicious + optional suspicious + optional
(malicious votes - harmless votes) - optional reputation`, compared inclusively
to the configured threshold. Signed arithmetic does not wrap. Missing required
data produces Unknown. A report below the threshold is labelled **Not flagged
at the configured threshold**, never a safety guarantee. Warning can suggest
typed `Decision::Reject` only when the configured recommendation is enabled.
The source actually hardcodes Reject despite its action field. The approved
native adaptation may emphasize Reject once for an untouched current popup;
it never auto-answers, changes the frozen timeout fallback or overrides user
interaction. Parent integration must match both session and object/prompt ID
before showing a live warning or recommendation.

Settings revision/generation are captured together under a short memory lock.
Accepted Save/Reload cancels active work and marks queued old work stale. Ticket
cancellation/drop prevents its result from being applied. Error/quota/excluded/
missing-key/no-data states stay Unknown. Native settings retain their staged
revision until Save/Reload, and opening a view never implicitly reloads/cancels
a current lookup. The captured-query view displays the exact supplied subjects;
reopening for a different capture must replace that inspection, not activate an
older one silently.

The integrated singleton view/backend/prompt code now passes a desktop check
(3.57 seconds) and strict no-default-features/all-target Clippy (3.76 seconds).
The initial compile failure is preserved in
`.verification/reputation-integration-check.log`; pinned TextareaState lacks the
validation API, so exclusions now use at most 32 validated 128-byte Kit Input
rows per category with Add/Remove, rather than an unbounded text editor.

The first seven focused cases pass with only owned loopback HTTP fixtures:
`.verification/reputation-20261016/core.log` and `network.json`. They cover source
formula/bounds/private exclusions, secret permissions/symlinks/stale preservation,
GET-only/cache/capture/redacted echo, 401/404/429/error/quota/disabled, queue cap,
held-HTTP Reload/stale/cancel/quit and body/timeout boundaries. Rendered/input and parent integration are now separately proven by the
15-frame real-GPU journey in
`.verification/headless/20261017T031615Z-c9807eec/result.json`; aggregate
`.verification/reputation-result.json` retains the full source snapshot, image
hashes, current combined core scope and exact boundaries. The journey proves
actual settings/refusal/reopen, exclusion, report/error/redaction/copy/cancel,
permission replies during held HTTP, untouched Reject once, late warning after
5-minute review, singleton capture replacement, old-result isolation and disable
clearing. The final advanced-details row is partly clipped in the collapsed
warning screenshot; decision controls remain visible and pass actual input.
Official API, real credential, upload/link and native compositor/package proof
remain outside this owned-loopback run. Source TODO file upload, automatic scanning and new malicious desktop
notifications are not shipped-source parity requirements.

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

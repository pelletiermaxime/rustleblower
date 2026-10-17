# Connection permissions

An unknown connection opens a separate permission window naming the app and
reported destination. Two Kit radio choices apply to the whole app or that app
at the exact reported hostname/IP, with any port. A plain-language preview sits
above Block (with its selected duration), Allow for 12 hours and Allow forever.
Fresh preferences default to whole-app scope. Saved scopes/conditions stay intact
and appear as Custom when they do not match a simple choice. Selecting a simple
choice explicitly clears additional conditions for that request. Missing executable
identity disables both simple choices.

Details and advanced rules… expands durations, full scopes, additional conditions,
priority/logging, Reject, selected-duration Allow and live request details. Conditions
already required by the selected scope are omitted from the checkboxes. Both
lasting Allow buttons remain available while expanded.

## Sub-features

- `permission-icons`: show a matching local desktop app icon beside its name,
  with initials while loading or when lookup fails. Background work never gates
  permission input. Run `python scripts/verify-headless.py --feature permission-icons`;
  require four JSON checkpoints, actual PNG/SVG loading/fallback assertions, cleared prior-request icons,
  an Allow reply while both icon workers are held, and owned cleanup. This proves
  fixture metadata and Kit/protocol behavior, not system-daemon app identification.
- `permission-allow`: click Allow forever or Allow for 12 hours and receive an
  allow/always or allow/12h rule in one click, regardless of the saved duration.
- `permission-deny`: activate Block with its displayed duration by pointer or keyboard.
- `permission-options`: choose a preset duration and target; combine up to 16
  additional conditions, including available checksums, with AND.
  The compact scopes are mutually exclusive. The destination choice retains the
  executable and matches an exact hostname, falling back visibly to an IP. It adds
  no port or protocol restriction. Advanced rule scopes retain the complete set
  of predicates; the connection scope still includes address, port, user and protocol.
- `permission-reject`: receive a reject rule with the chosen duration.
- `permission-details`: view/copy live arguments, environment, checksums and ancestry.
- `permission-checksums`: mismatch warning → Review saved checksums… → separate
  [checksum review](checksums.md). Updating rules never answers the permission.
- `permission-preferences`: main window → Popup preferences… → default duration,
  target, priority/logging, Show permission popups → Save preferences. Changes
  apply to requests admitted afterward and persist beside the selected database.
  A missing target falls back to executable + destination. Disabled popups reply
  immediately once using the configured fallback; existing requests remain valid.
  Conditions & timeout adds default IP/port/UID/checksum conditions, initial
  expanded options, 1–110 seconds and allow/deny/reject once. Blank timeout and
  Use launch setting inherit CLI values; saved overrides take precedence.
  Missing requested fields expand the options with a warning for explicit review.
- `permission-lasting`: lasting replies await daemon observation in Application
  rules; local decision history cannot confirm them. Refresh connection obtains
  a fresh inventory, including unlogged or expired rules.
- `permission-timeout`: unanswered requests use the explicit fallback.
- `permission-review`: pointer/keyboard input in the permission popup pauses its
  ordinary countdown. Initially expanded options also pause it on display. The
  hard request deadline remains visible, capped at 120 seconds from admission
  minus the reply margin. Other queued requests keep their own countdowns.
- `permission-placement`: Popup preferences… → Placement → Center/four corners.
  X11 uses the primary monitor/workspace and native outer-frame size. Native
  Wayland disables the selector and explains compositor control. A request keeps
  its captured initial position; expanding/manually moving does not re-anchor it.
- `permission-lifecycle`: cancellation, reconnect, and shutdown invalidate pending requests.
- `permission-overload`: more than 32 pending requests receive a one-time fallback.

## How to get to it (user POV)

- A subscribed daemon calls AskRule for a connection it needs the user to decide.
- The first request opens the permission window; subsequent requests queue there.
- Closing the permission window minimizes it and leaves the countdown running.

## Driving it with GPUI input and native controls

Preconditions: run from the root, with an owned socket/database and synthetic
requests; never generate real firewall traffic just to produce a prompt.

- **Background click and keyboard.** Run `python scripts/verify-headless.py`.
  Require `05-permission.json`, `06-permissions-complete.json`, and a passing flow.
  It clicks `allow-forever`, then uses Tab/Space on the next request and checks the
  backend replies. This is UI input with fixture requests, not daemon enforcement.
  Frames 35–40 exercise Forever + command/user/port/checksum + priority/no logging,
  Reject over the real gRPC server, live-detail copying, pending-to-observed rule
  promotion, reset to saved duration/default options, and a five-minute network Allow.
  Require all six JSON checkpoints and the successful wire/cache assertions. Wheel events
  reach lower conditions; the fixture never sets the choice directly.
  Frames 41–43 save a 12-hour executable default with priority/no logging, verify
  those fields in the next real gRPC reply, then disable popups and require
  immediate deny/once without a queued prompt. The focused
  `python scripts/verify-headless.py --feature popup-preferences` separately
  drives the controls and verifies disk reopening with thirteen JSON checkpoints, including
  invalid-timeout feedback, advanced settings, five initial popup bounds and a
  queued corner-to-center transition. Production-window Tab/Space is also checked.
  Frames 44–48 save four conditions and Reject fallback, verify the actual
  resulting RPC rule, display unavailable default fields, and expire a one-second
  request to Reject once despite a lasting duration selection.
- **Simple scopes and direct actions.** Run `python scripts/verify-headless.py --feature permission-actions`.
  Require six JSON checkpoints, actual allow/always and allow/12h replies, app-only scope,
  explicit clearing of saved address/port restrictions, exact hostname and IP
  scopes without port/wildcard, keyboard radio selection, preserved custom
  conditions/metadata, disabled simple choices for unknown executable, unchanged
  saved preferences, six SQLite decisions and scoped cleanup.
- **Foreground reply.** Run `python scripts/verify-native.py run --feature prompts`.
  Add `--decision allow` for the Allow path. Require deny/once or allow/always
  fixture reply before ten seconds, screenshots, and all
  cleanup booleans true. The 30-second fallback cannot satisfy this proof.
- **Native X11 placement.** Run `python scripts/verify-native.py run --feature placement --backend x11 --position bottom-right`.
  Repeat for center/top-right/top-left/bottom-left. Require actual visible popup
  bounds in `geometry.json`, frame extents and primary workspace, error within
  two logical pixels (minimum three physical for rounding), an input decision
  before fallback and all cleanup booleans. The verifier only reads position.
  Native Wayland permission proof does not imply Wayland corner support.
- **Timeout/cancellation/overflow.** Run `python scripts/cargo.py test --no-default-features --locked`.
  Require the named protocol tests to pass, including client deadline/shutdown.
  This proves wire replies and stale-ID rejection separately from native controls.
- **Countdown review.** Run `python scripts/verify-headless.py --feature countdown`.
  Require four JSON checkpoints, actual Allow/Deny RPC replies and persisted input decisions.
  Pointer opening and keyboard Tab must each keep a two-second request pending
  beyond its ordinary timeout; the next request must start with a fresh timer.
  The full flow separately checks initially expanded review and ordinary expiry
  when advanced display is disabled. Core tests verify a reviewed request expires
  before a shorter RPC deadline, keeps its captured fallback despite preference
  changes, and does not pause another queued request. Cancellation/reconnect and
  shutdown tests include already reviewed requests.
  For native proof run `python scripts/verify-native.py run --feature countdown`.
  Tab must show Reviewing/paused after 5.2 seconds; Space must produce the actual
  configured reply, persisted as a decision. Require every cleanup boolean.

## Gotchas

- Timeout, overload, cancellation and shutdown fallbacks remain once-only even
  when a lasting duration was selected. Invalid/expired/stale choices cannot
  answer a replacement request. The core choice tests cover racing decisions,
  all duration/action combinations, derived regex/network matches and allocation limits.
- Each request captures its defaults on admission; extra conditions reset.
  Saved X11 placement has native geometry/input proof on the tested KWin/XWayland
  setup. Native Wayland retains compositor placement. Checksum review has its
  own feature/proof map; native update controls remain pending there.
  Deadlines and fallback actions also stay captured through later settings saves,
  reconnects and shutdown. Core protocol tests distinguish old Reject/new Allow
  overrides and require a short RPC deadline to cap a saved 110-second timeout.
- Preference-file reads are bounded to 4 KiB. Core tests cover persistence,
  invalid/future/oversized files, failed-save memory preservation, missing targets,
  immediate disabled-popup replies and existing-request isolation.
- Review pauses the ordinary countdown, never the hard request limit. Mouse
  movement alone does not pause it; pointer press or keyboard input does. Closing
  or minimizing a reviewed window does not restart its original countdown.
- A lasting reply is not an installation ACK. Pending entries cannot be edited
  or bulk-mutated until reported by the daemon or replaced by a fresh inventory.
- Advanced options have Kit input and synthetic gRPC proof; native
  compositor input and installation by the real daemon remain unverified.
- The compositor can refuse window activation. Fallback remains independent of UI.
- Tab needs an initial focus handle and a rendered frame before activation.
- A decision in history records the chosen reply, not daemon enforcement.
- Native close/minimize and restoration are source-backed entry points; the
  current automated foreground flow does not assert those compositor behaviors.

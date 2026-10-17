# Missed permission popup notifications

The reference is OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`, particularly
`ui/opensnitch/config.py:133-138` and `service.py:637-688`.
Rustleblower sends a best-effort desktop notice when an unanswered permission
request reaches its ordinary timeout. Its action opens Application rules so the
user can review the fallback and allow an application manually.

Popup preferences has a Missed popups tab with an independent toggle, template,
preview and reset. The general desktop-notification toggle also applies. The
template defaults to the reference's connection/rule/node message. Known fields
include connection addresses, ports, protocol, executable, working directory,
arguments, PID and UID, the applied rule action and node address/name. Unknown
tokens remain literal. Expansion is one pass: connection data cannot inject new
template tokens. Text values have control characters replaced with spaces and
the notification adapter escapes desktop markup.

The pinned Kit multiline state lacks a public validator. The bounded single-line
input therefore accepts reversible `\n`, `\t` and `\\` escapes; the preview shows
their actual expansion. Incomplete/unknown escapes refuse the whole staged Save.
The stored template remains ordinary text, at most 1,024 UTF-8 bytes; input is
at most 2,048 bytes, rendered body at most 512 bytes and title at most 128 bytes.
Truncation never splits a character or resumes with a later suffix.

The fallback rule is captured at request admission. Notice preferences are read
at completion so disabling notices or editing their text takes effect for an
already pending request. Only the actual timeout produces a notice. Manual
answers, overload, disabled popups, reconnect/disconnect and shutdown do not.
The trusted transport identity determines the remote address; the daemon's
reported hostname is display text. A reported hostname cannot make a remote peer
local.

Permission completion performs a short settings snapshot, bounded formatting
and nonwaiting insertion into the shared eight-entry mailbox. It does not wait
for telemetry, SQLite, a visible window, D-Bus or delivery. Queue saturation
preserves the permission fallback and records the skipped best-effort notice.
The desktop action never submits a rule or answers another request.

The preference document is version 5, retaining its existing 4-KiB limit.
Versions 1–4 receive the default template and enabled per-type toggle. Invalid
templates preserve the previous saved file.

Verification is recorded in `.verification/missed-popups-result.json`:

- Four dedicated core cases and four preference regressions cover token handling,
  migration, validation, bounded queues and real owned gRPC timeouts. Captured
  Reject fallback, latest preferences, hostname, immediate mute and manual-answer
  silence are asserted independently of the GUI.
- A private D-Bus service verifies exact body/actions, rejected stale actions,
  notification replacement and Rules routing. Service acceptance is distinct
  from a painted host desktop banner.
- The 16-frame GPU journey under
  `.verification/headless/20261017T024155Z-6f0736eb` uses actual controls to edit,
  save, reopen, mute, refuse invalid escapes and reset the template. It checks a
  synthetic hidden-window Rules callback and answers a second real permission
  request through input. Actual timeout production and D-Bus delivery are proved
  by the separate layers above.

Later update-checker routing adds another notification target; its proof is
separate. Exact scoped source hashes accompany the reports. No system daemon,
production socket, host desktop banner or physical notification click is used.

# Rustleblower

Build a small native replacement for opensnitch-ui while preserving the existing
OpenSnitch daemon and its policy ownership. FerryDock is the architecture and
verification reference at `/home/maximep/projects/ferrydock`.

- Check the pinned GPUI Kit components before custom rendering; keep one GPUI backend.
- Keep feature state/views together, I/O off the UI thread, queues/payloads/pages bounded.
- Permission handling must not wait on telemetry ingestion, SQLite, or main-window visibility.
- Never replace an existing socket or change the system daemon to run verification.
- Use [.agent/skills/verify-rustleblower/SKILL.md](.agent/skills/verify-rustleblower/SKILL.md)
  and its feature map. Prefer the background GPU flow while the user works;
  use owned foreground sessions for compositor behavior.
- Preserve ignored `.verification/` evidence, clean up only owned processes/sockets,
  and distinguish build, input, protocol, persistence, and real-daemon proof.
- Keep the README user-focused and development procedures under `docs/`.

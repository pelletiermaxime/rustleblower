# Connection and application lifecycle

Rustleblower listens for local daemon sessions, shows its heartbeat/version/rule count,
and keeps serving permissions when its main window is minimized.

## Sub-features

- `connection-subscribe`: daemon configuration is echoed unchanged.
- `connection-heartbeat`: Ping is acknowledged quickly and refreshes status.
- `connection-alert`: the latest daemon alert appears in the dashboard.
- `connection-reconnect`: same-name replacement invalidates only that node's old requests/commands; distinct names retain independent sessions.
- `connection-owned-socket`: existing endpoints are never overwritten.
- `lifecycle-minimize-quit`: closing minimizes; Quit Rustleblower stops the server.

## How to get to it (user POV)

- Launch Rustleblower from the desktop or terminal.
- The daemon connects to the configured Unix socket automatically.
- Read the dashboard status; use the window close control to minimize or the
  explicit Quit Rustleblower button to exit.

## Driving it with native controls and protocol tests

Preconditions: use the helpers' isolated socket and database; leave system
opensnitch-ui and opensnitchd alone.

- **Connection display.** Run `python scripts/verify-native.py run --feature history`.
  Inspect the populated dashboard's synthetic daemon version/name and require
  successful Ping RPCs in `fixture-report.json`.
- **Protocol lifecycle.** Run `python scripts/cargo.py test --no-default-features --locked`.
  Require subscription/ping/alert, reconnect, stream-close, shutdown, and existing
  endpoint protection tests to pass. `tests/uds_authority.rs` also verifies the
  daemon's raw socket-path authority and C-core's encoded form through actual
  HTTP/2 frames. Socket cleanup is asserted after joining.
- **Compositor lifecycle.** Close/minimize restoration and the Quit button are
  currently source-backed manual paths; do not claim native proof from a helper
  that terminates its owned process during cleanup.

## Gotchas

- Protocol regressions send Ping before Subscribe, verify that early events are
  persisted under the subscribed name without a loss warning, and replay them
  to check deduplication. They also check the 32-batch startup bound, disconnect
  cleanup, and rejection of reports from a superseded session.

- The main window is deliberately not disposable while the server is running.
- No tray icon/autostart entry is provided yet. A crash can leave a stale socket;
  establish ownership before removing any endpoint manually.
- Local Unix sockets only. Up to 32 distinct named sessions are supported;
  TCP/TLS and remote peer identity remain outside this slice. See [Nodes](nodes.md).
- A live server with stale daemon heartbeat is different from an active daemon.

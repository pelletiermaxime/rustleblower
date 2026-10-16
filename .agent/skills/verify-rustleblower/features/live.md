# Live sockets, process details and node resources

## User behavior

Open Live or Ctrl+6, choose Sockets, Process details or Node resources, then
Start monitoring (Ctrl+Enter). Pause is Ctrl+Shift+Enter. The target is the node
currently selected through Nodes. Socket filters and interval are staged while
paused and apply at the next Start. Available filters match the installed UI:
protocol, family and TCP state, with 1/2/5/10/30/60-second intervals. Permission
prompts remain independent of task replies. No control changes firewall policy.

Sockets show endpoints, protocol/state/family, PID/UID, executable/command,
interface, inode, receive/send queues, timer, expiry, retransmits, mark and cookies.
Search filters the retained snapshot, with 200 rows per page. Pause freezes it for
stable selection/paging/copy. Selecting a socket enables copying its complete
original JSON and inspecting its known PID. Unknown PIDs cannot start a process
task. Process mode also accepts a manually entered positive Linux PID.

Process fields include status, descriptors, I/O, maps, stack, environment,
arguments, tree, checksums and memory, plus all reported unknown fields. Select a
field, search its readable lines and copy either its complete content or the full
JSON. Node resources display reported Sysinfo fields and a summary of uptime,
process count, fixed-point load averages and memory/swap using the reported Unit.
This daemon task does not report CPU utilization or per-application bandwidth.

Leaving Live, closing the dashboard, changing mode/target, pausing or dropping the
inspection requests stop. Closing with no tray uses the current minimize fallback
and still requests stop. Direct titlebar minimization alone has no proven stop
hook. Retained snapshots keep their source node and are marked stale on disconnect;
Start uses the currently selected live generation. Reconnect never silently
resumes a task. Failures preserve the last displayed data and show an error.

## Source contract

Pinned OpenSnitch v1.7.2:

- [Task command handlers](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/ui/notifications.go): TASK_START=13, TASK_STOP=14, JSON task names/configuration.
- [Streaming replies](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/ui/notifications_tasks.go): repeated results share the original TASK_START notification ID.
- [Socket dump](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/tasks/socketsmonitor/dump.go) and [kernel fields](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/netlink/socket_linux.go): Table, Processes, IP strings, numeric fields and cookies.
- [Process data](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/procmon/process.go) and [node resource task](https://github.com/evilsocket/opensnitch/blob/v1.7.2/daemon/tasks/nodemonitor/main.go).

Installed `dialogs/stats.py::_monitor_node_netstat`, `_unmonitor_node_netstat`,
`_update_netstat_table` and `dialogs/processdetails.py` define the UI workflows.
Sockets config includes interval/state/proto/family; PID is a string; resources
use node plus interval. Stop reuses the same identity/configuration. A successful
TASK_STOP has **no reply** in this daemon. State says Stop requested, not confirmed
stopped. Read-task errors do not invalidate the confirmed firewall-rule inventory.
Daemon disconnect stops daemon-owned tasks; no UI process reads host /proc or
kernel sockets directly for remote inspection or synthetic verification.

## Bounds and architecture

Four tasks globally; one running inspection in this dashboard; one latest raw
snapshot per task, at most 1 MiB each. Repeated updates replace a watch slot rather
than extending an event queue. Task IDs share the policy command allocator but
never consume its sixteen one-shot pending slots. Commands use the existing
bounded eight-message stream. Full streams retain a bounded owned stop retry,
retried by backend maintenance, and prevent duplicate task identities. Lack of
updates for three intervals plus ten seconds requests stop with explicit error.
Session generation and task ID must both match; late/cross-node replies are ignored.

RPC routing checks size and replaces a raw slot; JSON parsing stays on one shared
background parsing permit. Socket arrays and process indexes retain at most
10,000 entries, with an explicit shortened flag. Parsed socket view data is capped
at 4 MiB including raw bytes, row allocation and string capacities. Process paths
are decoded once per PID and normalized into bounded summaries. Borrowed raw row
ranges keep unknown fields available for copy. Unknown protocol/state/family
numbers remain visible. IP endpoint formatting brackets IPv6 correctly.

Generic fields retain up to 128 names; previews retain 2,048 lines of 512 bytes.
Map/array previews use bounded raw visitors, preserving escaped names and exact
numbers. Full copy retains the complete admitted original payload/selected field.
The latest live snapshot is volatile, separate from disk-backed connection history;
there is no growing live-snapshot archive. Failed or shortened data is never
silently described as an exhaustive kernel view. Kernel/daemon limits still apply.

## Verification

- `python scripts/cargo.py test --no-default-features --locked`: actual isolated
  gRPC streaming, 25 updates under one ID, crossed node/stale generation rejection,
  no stop ACK, errors/oversized payloads, task bounds/stop retry/stale expiry and
  independent Ping/Allow within 500 ms. Model proof covers nulls, IPv6, queues,
  unknown values, raw preservation, escaped names, preview caps and Sysinfo units.
- `python scripts/verify-headless.py --feature live`: eleven screenshots; actual
  Kit Start/Pause/Select/paging/selection/search/copy, two replies per start,
  225 sockets and a 113-row filtered result, PID details/environment/descriptors/
  maps, retained data on ERROR, permission input while a resource update waits,
  resource summary, five starts/five stops including dashboard close/restoration and zero policy commands.
- Rebuild desktop after core, then
  `python scripts/verify-native.py run --feature live [--backend x11]`: owned
  synthetic Python task fixture, actual Ctrl+6/Start/search/Pause, painted rows,
  empty search and Stop requested, exact TASK_START/TASK_STOP JSON, 430 history
  rows and all cleanup flags. The fixture never invents a successful stop ACK.

Native socket/process enforcement is not a concept of this read feature. Synthetic
proof does not establish real kernel/process visibility, permissions on the user's
daemon, native process-field/picker/paging/copy controls, long-run RSS, exports,
persisted filters/columns or native history-to-live-process shortcut input. The shortcut itself is implemented;
see the focused history-process proof below. These remain
explicit parity/proof gaps; the broader goal is not complete.

## History-to-process inspection

History → select connection → Details → Inspect process opens Live process details
and immediately requests current data for the recorded PID on that event's daemon.
It selects that daemon explicitly, rather than using an unrelated node currently
selected in Nodes. Unknown/nonpositive/out-of-range PIDs, old events lacking
metadata and disconnected nodes cannot start an inspection. A queued request
captures the session generation; reconnecting before dispatch rejects it.

This reads the current PID. The original process may have exited and a PID may
have been reused. The details window explains this before the action; Live keeps
the recorded event/time/path/node alongside an explicit current-data warning.
A task error retains its last displayed data and shows the error. Leaving Live
requests stop through the existing lifecycle. No historical environment, maps or
checksums are invented. Inspection needs a notification stream, not a ready rule
inventory. Input queues retain at most four bounded requests and one per window.

Source: installed OpenSnitch 1.7.2 `dialogs/stats.py::_set_in_detail_view` PID branch
and `_cb_proc_details_clicked`, plus `dialogs/processdetails.py::monitor`.

`python scripts/verify-headless.py --feature history-process` drives the history
selection/details/button with actual GPUI input and two live gRPC fixtures. Require
eight screenshots, exact PID42/five-second task only on history-alpha while a
different selected node receives nothing, independent permission input while the
process reply waits, a different current executable in clipboard, process-exited
error retention, refusal of unknown/invalid/offline targets, and stale-session
rejection after reconnect. Native shortcut input and real process visibility
remain separate unverified boundaries.

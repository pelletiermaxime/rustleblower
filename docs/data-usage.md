# Data usage

Rustleblower 0.4 includes **Data usage** (`Ctrl+8`): live upload/download rates and
totals per executable, with application search, numeric column sorting and
100-row pages. Totals cover the current collector session. Closing the main
window to the tray or switching pages keeps collection running. **Stop** closes
the receiver and removes only its own socket.

The desktop app remains unprivileged. The independent Linux collector measures
traffic; it does not depend on KDE, modify OpenSnitch or own firewall policy.
**Start monitoring** launches the collector through the system authentication
dialog using `pkexec` (polkit). The app never receives your password. The
AppImage, Debian package and archive include the collector and its shared
libraries; no separate collector build or manual command is needed.

## Start monitoring

Run the installed app as your desktop user.

1. Open **Data usage**, then **Start monitoring**.
2. Approve the collector in your desktop's system authentication dialog.
3. The page changes from **Waiting for system authorization** to **Monitoring**
   when measurements arrive. Rates appear after the second sample.

This requires `/usr/bin/pkexec` and a registered desktop polkit authentication
agent. It uses the existing desktop agent, including Plasma's agent; it does not
link to or depend on KDE libraries. There is no terminal password fallback and
no password field in Rustleblower. Cancelling authorization shows
**Authorization cancelled** and lets you retry with Start. Missing helper,
missing agent and load failures appear explicitly; no zero-traffic result is
substituted. Startup has a two-minute deadline. Start is disabled while a
collector or authorization request is still owned by the worker.

The running kernel must provide BTF, BPF LSM enabled in its active LSM list,
the required TCP/UDP tracing hooks and the `bpf_get_task_exe_file`,
`bpf_path_d_path` and `bpf_put_file` filesystem kfuncs. Missing support produces
an explicit unavailable status. The current kernel proof is on CachyOS 7.2.9;
packaging on Ubuntu 24.04 does not establish support for Ubuntu's default kernel.

The temporary helper needs elevated BPF access; the GUI does not. The prompt
authorizes the collector executable. No passwordless authorization rule or
persistent service is installed. The helper verifies the receiver UID, and the
receiver accepts only its own user or root. Collection covers the current
desktop user's processes. Each invocation lasts at most one hour; Start begins
a fresh session afterwards.

**Stop** cancels pending authorization or closes an active connection, causing
the helper to exit and release its unpinned probes/maps. Child reaping happens
off the UI thread. A collector disconnect preserves the last totals with rates
unavailable. Closing the window to the tray keeps collection running; Quit
requests collector shutdown too.

**Copy collector address** remains available for diagnostics; normal startup
does not require copying it or running a terminal command.

## Collector installation and source builds

Installed packages use `libexec/rustleblower/{collector,accounting.bpf.o,lib/}`;
archives keep this directory alongside the desktop executable. Before
authorization the worker copies at most 32 MiB into a new private temporary
directory, including the collector's dynamically linked libraries. This avoids
root access to a user AppImage FUSE mount. The copy stays alive until the helper
exits and is then removed. Files must belong to this user or root and disallow
group/other writes; symlinked helper, object and library files are refused. The
temporary filesystem must allow executable files. Library notices and
replacement details accompany the bundle.

Build both components as your normal user from the repository root:

```sh
python scripts/cargo.py build --locked
python scripts/bandwidth/build.py
```

The collector builder needs `clang`, `bpftool`, a C compiler, `pkg-config`, libbpf
development files and readable kernel BTF (`/sys/kernel/btf/vmlinux`). It prints
the absolute build directory, retaining its log, object and manifest there. It
does not load BPF or install a service. A successful build atomically updates the
local `target/bandwidth` link so the source UI can find that exact helper/object
pair. `--no-stage` leaves this link unchanged for isolated verification builds.
The default build checks this kernel's hook signatures. `--portable` builds the
same CO-RE collector using the packaged type subset and bundles its libraries,
without requiring `bpftool` or the build host's kernel features. At startup,
libbpf relocates the object against the running kernel; missing hooks, fields
or kfuncs fail loading explicitly.

Use the newly built desktop binary as your frontend, following the README's
existing-socket precautions. An already running frontend retains its socket;
do not replace that socket to try this view.

## Reading the numbers

These are **socket API payload bytes**: successful sends accepted by the kernel
and bytes copied by receives. They exclude network headers, ACK packets and
retransmission overhead; totals are not an ISP/interface meter or proof of
remote delivery. UDP can carry QUIC, but this prototype's real protocol proof is
TCP/UDP workloads rather than a browser QUIC session.

- Rates use monotonic sample intervals. The first sample has totals but no rate
  until a second sample arrives. After a truncated snapshot or a new loss,
  rates wait for two consecutive complete snapshots; retained totals stay visible.
- Several processes running the same executable contribute to the same row.
  Process lifetime identity prevents a reused PID from resetting old counters.
- Executable paths are captured during socket operations and retained after
  process exit. Quick commands such as curl combine into one executable row even
  when they finish between snapshots. Process/execution/file identities prevent
  unrelated applications from being merged just because their names match. When
  capture fails or the bounded metadata table is full, the row explicitly says
  **path unavailable**.
- A collector that stops updating for three seconds shows **rates unavailable**.
  Disconnects, malformed data and incomplete collection are visible. Missing
  collection never becomes a zero-traffic result.
- Search and numeric sorts apply across all records before paging. The total
  uploaded/downloaded counters cover the whole session, including filtered-out
  applications.

There is no daily/monthly persistence, destination breakdown or remote collector
in this first slice. Shared sockets, containers, real QUIC, other kernel paths
and sustained overhead still need their own proof. See
[measurement semantics and remaining gates](bandwidth-plan.md).

## Verification

The user-run kernel test is separate from UI and transport checks:

```sh
sudo /usr/bin/python -I scripts/bandwidth/verify.py --build /absolute/build-directory
sudo /usr/bin/python -I scripts/bandwidth/verify.py --short-lived --build /absolute/build-directory
python scripts/bandwidth/test_transport.py --build /absolute/build-directory
python scripts/verify-headless.py --feature bandwidth
```

The kernel test creates an owned private network namespace, targets only its
workload PID/UID and checks exact TCP/UDP bytes, peek/truncation and post-exit
retention. The short-lived variant gates three real curl processes, filters
only their PIDs, then requires exact per-process bytes and retained kernel paths
after all three exit before the first usage sample. The transport test exercises the actual C connector without BPF or
sudo, including receiver-UID/path refusal. The Kit journey uses a real Unix
stream fixture and actual input to check controls, rates, sorting, paging,
reconnect, malformed-input retention, an independent permission RPC during a
held frame, authorization cancellation/retry, Stop during authorization and
scoped cleanup. Authorization is replaced by an owned child fixture in those
tests; a real desktop password prompt and elevated end-to-end startup require
their own desktop-session check. It does not establish real fonts, compositor
input, packaged installation or whole-machine accounting accuracy.

All network/socket checks must run outside a sandbox that blocks their owned
local sockets; never substitute the real daemon socket or host policy.

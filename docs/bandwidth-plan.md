# Per-application data usage

The user selected live rates and session totals on October 17, 2026. This is a
new Rustleblower feature; connection-count Statistics and Live socket inspection
do not provide these measurements. The pinned OpenSnitch protocol has no
transferred-byte counters. OpenSnitch retains policy and permission ownership.
The user explicitly ruled out a KDE dependency. Collection must work independently
of KDE services, helpers and libraries.

## First version

A native Data usage view now shows executable, download/upload rate and
downloaded/uploaded totals, search and numeric sorting, with totals labelled
**since monitoring started**. Collection health, stale data and incomplete
accounting must remain visible; unavailable collection is never zero traffic.
Use the pinned GPUI Kit Table and, where useful, its charts. Keep one GPUI backend.

Daily/monthly persistence, destination breakdown, remote collection and network
quotas are later work. The source-build UI launches the standalone collector through `pkexec` and
the desktop authentication agent; no service or authorization rule is installed. See [data usage setup](data-usage.md).

## Measurement prototype

`scripts/bandwidth/build.py` builds a small standalone libbpf loader and CO-RE
eBPF object against the current kernel's readable BTF. It validates the six accounting hook layouts, two socket LSM layouts and
required filesystem kfuncs and refuses other layouts rather than dropping protocol support
silently. Generated BTF/header, binaries, logs and manifests stay in ignored
`.verification/bandwidth/` directories. It does not load code or install anything.

The loader attaches entry/exit hooks to `tcp_sendmsg_locked`, `tcp_recvmsg`,
`udp_sendmsg`, `udp_recvmsg`, `udpv6_sendmsg` and `udpv6_recvmsg`. Two additional
BPF LSM hooks at `socket_sendmsg`/`socket_recvmsg` capture executable metadata
for IPv4/IPv6 sockets. They always return the prior security result, preserving
existing decisions even when metadata capture fails. Collection is enabled only
after all fourteen programs attach. It requires an explicit UID and can restrict
itself to up to eight explicit PIDs (repeated `--pid`) for isolated verification. Partial attachment failures unwind owned links;
there are no pinned maps, firewall changes or system services. Its optional
`--connect` stream connects to the unprivileged app's owned Unix socket, verifies
the receiver UID and applies a two-second send timeout. Stdout remains available
for isolated measurement verification.
On the installed optimized kernel, both `inet_sendmsg` and `inet6_sendmsg` call
`tcp_sendmsg_locked` directly. Hooking only the `tcp_sendmsg` wrapper missed those
sends despite successful attachment. The actual vmlinux disassembly is retained
with the verification evidence; the locked implementation is hooked once to
avoid counting both wrapper and implementation returns.

Counters mean **socket API payload bytes**: successful sends accepted by the
kernel, and bytes copied by receives. They exclude Ethernet/IP/TCP/UDP headers,
ACK packets and retransmission overhead. This is not an ISP or interface-level
usage meter, and a successful TCP send is not proof of remote delivery. UDP
traffic can include QUIC's encrypted protocol bytes; the collector does not parse
QUIC or inspect packet contents.

Count the positive return value, capped at the requested buffer size. Failed
calls, zero returns and receive `MSG_PEEK` do not add bytes. Receive `MSG_TRUNC`
can return a full datagram length while copying fewer bytes, so only the buffer
size counts. Per-thread/per-direction call depth suppresses double counting
when IPv4-mapped IPv6 UDP forwards through an IPv4 hook.

The key includes host TGID, process-leader boot start time, execution ID, UID,
executable device/inode/mount ID and leader comm. PID reuse and replacement by
another execution retain distinct counters and metadata, even if an inode is
recycled. Missing lifetime/executable identity counts as loss.

At the first socket operation for an identity, the metadata hooks acquire the
current executable file with `bpf_get_task_exe_file`, resolve its full path with
`bpf_path_d_path`, then release the file with `bpf_put_file`. An immutable bounded
map retains the captured path, so curl can start, transfer and exit entirely
between two samples and still contribute to one executable's total. There is no
later `/proc` lookup or name-based merging. Failed/overlong path capture remains
explicitly unknown. Grouping includes executable identity and captured path;
unknown records stay separate instead of borrowing a known app label. A C check
covers bounded path validation and hex encoding; live-kernel tests prove actual
path capture. This is not security-grade executable verification or browser-tab
identification. The current collector requires kernel BPF LSM and these kfuncs;
missing support fails build/load explicitly.

Kernel maps cap cumulative byte identities and metadata identities separately at
4,096 each, with paths limited to 512 bytes plus NUL. The metadata map adds about
2 MiB of path bytes plus key/map overhead. Scratch path storage is one fixed
513-byte buffer plus length per CPU; it is not on the BPF stack. In-flight calls
remain capped at 8,192. Metadata pressure or capture failures leave the path
unavailable while byte counters can continue.
No LRU eviction silently discards closed-process totals. Map/identity failures
increment a loss counter. Each JSON-line snapshot is capped at 2 MiB, with explicit
truncation if it cannot include every identity. Paths/comm use hex encoding to
avoid injecting control characters or invalid UTF-8 into the stream. Polling is
100–5,000 ms; an invocation lasts at most one hour. The loader and kernel maps
are separate from GPUI, SQLite, OpenSnitch RPCs and permission handling.
Five bounded diagnostic counters record matched entries/returns, missing
in-flight calls, positive non-peek returns and accounted calls. They use the same
UID/PID filter and retain no additional application or payload metadata.

## Collector choice

[Picosnitch](https://github.com/elesiuta/picosnitch) is a candidate for reuse:
it already monitors per-executable traffic and exposes a live event socket.
Its current documented live socket requires root and its runtime includes
hashing, persistence and notification facilities beyond this feature. Assess a
supported unprivileged read boundary and accurate live rate semantics before
selecting it. Do not install another service just to compare the designs.

[OpenSnitch's experimental bandwidth branch](https://github.com/evilsocket/opensnitch/issues/1081)
is useful reference material, but changing/forking the existing daemon is outside
this feature. The standalone probe is a feasibility experiment, not a settled
replacement for an existing collector.

## Verification and privilege boundary

Build as the ordinary user:

```sh
python scripts/bandwidth/build.py
```

Use the absolute directory printed by the builder. The live test needs root
because this machine disables unprivileged BPF. From the user's terminal:

```sh
sudo /usr/bin/python -I scripts/bandwidth/verify.py --build /absolute/printed/build-directory
```

The verifier checks the build manifest's kernel and object hashes, creates a
private network namespace, enables only that namespace's loopback, and runs an
unprivileged workload child when invoked through sudo. The collector restricts
itself to that UID/PID, so it does not collect unrelated applications. Exact
sent/received assertions cover TCP and UDP on IPv4/IPv6, mapped UDP, TCP peeking,
UDP peek/truncation, an idle interval and retention after a process exits. It
checks only snapshots timestamped after each completed workload. An early socket
or worker failure fails the run. Every owned child is reaped and unpinned BPF
links/maps are released when the loader exits; evidence is retained and returned
to the invoking user's ownership.
The report also retains the current case/stage, last workload result, exact
expected/observed totals, last collector snapshot and sample count on failure.
Worker stderr and a failure traceback are preserved separately. A lack of
collector output remains distinguishable from repeated snapshots with missing
byte totals. Regression checks for these diagnostics run with
`python scripts/bandwidth/test_verify.py` and do not load BPF.

The workload harness can be checked without BPF/root:

```sh
python scripts/bandwidth/verify.py --workloads-only
```

That mode uses a private mapped-user/network namespace. A sandbox that blocks
netlink inside that namespace needs the normal execution approval; do not fall
back to the host network. A workloads-only pass proves test traffic generation,
not BPF loading or byte attribution.

The user's `.verification/bandwidth/run-20261017T193120934383Z/result.json` passes
all nine live-kernel cases with exact bytes, executable attribution, no loss and
owned process cleanup. The newer short-lived-process collector additionally
passes the original nine cases and three real curl processes that exit before
the first nonempty sample; all paths are retained from kernel capture.
Remaining collection gates include concurrent traffic, partial/failed sends,
shared sockets and process handoff,
real QUIC traffic, container/network/PID/time namespaces, counter pressure,
io_uring and other socket paths, suspend behavior, overhead and resource/exit
measurements. Kernel hooks and privileges need compatibility assessment on
other supported kernels. Do not claim complete accounting for untested paths.

## Native UI integration

`src/bandwidth.rs` owns the typed model; its worker owns all socket I/O, parsing,
application grouping and global search/sorting. There is one latest query and
one latest immutable 100-row page. Frames are at most 2 MiB; the session retains
at most 4,096 process identities. An invalid frame is refused before mutation.
UID, identity, duplicate, session, timestamp and counter checks preserve the last
good data. Captured paths survive process exit; records without captured paths remain
explicitly unattributed. A per-execution ID prevents reusing earlier metadata
after exec, while repeated processes of the same executable combine their totals. Rates are unavailable before a second sample or while
stale/disconnected/stopped. Partial input has a two-second completion deadline;
three seconds without a valid update makes rates stale.

Start binds a unique 0600 Unix socket in the user-owned database directory,
refusing unsafe directories and existing paths. Accept only kernel-authenticated
root/current-user peers and current-user records. Stop/Drop closes the stream;
only an endpoint with the recorded device/inode is removed. A replacement file
is preserved. Hidden-window collection does not depend on SQLite or a daemon
command, and quit requests independent worker shutdown. Start resolves the staged helper
off the UI thread and launches `/usr/bin/pkexec --disable-internal-agent` with
explicit arguments and no shell. The desktop agent owns credentials. Startup is
bounded to two minutes, stderr retention to 4 KiB and reads to 16 KiB per tick.
Cancellation (126), authorization failure (127), collector failure and normal
session end have distinct states. Stop closes the receiver before killing or
reaping the owned child on the worker; it does not block permission handling.

The pinned Kit Table supplies header sorting and virtualization; one page has
100 application rows. Ctrl+8 opens Data usage, Ctrl+F focuses its search. Last-page
window restoration includes the new page. The thirteen-checkpoint Kit journey covers
input, stream fixture, an independent permission RPC during a held frame and
scoped cleanup. Native fonts, compositor input, packaged helper installation, an actual desktop authentication prompt
and a real collector-to-rendered-desktop session still need their own proof.


## Release packaging

Version 0.4.0 bundles the standalone collector, CO-RE object and replaceable
shared libraries in all three Linux packages. The portable builder uses only
the kernel types/fields it accesses, without build-host BTF; runtime loading
remains all-or-nothing. The launcher makes a mode-0700 private copy before
calling pkexec, bounded to 16 libraries, 16 MiB per file and 32 MiB total. This
keeps root independent of user FUSE mounts and sanitized library environment
variables. Helper/object/library symlinks and group/other-writable files are
refused. The copy is held until the child exits, with cleanup off the UI thread.

The portable object passes all nine byte/attribution cases and three short-lived
curl cases on CachyOS 7.2.9. Disposable Ubuntu 24.04 build/loader checks cover
bundled library discovery and notices without claiming kernel support there.
Clean package checks run the packaged collector without LD_LIBRARY_PATH and
verify its object, libraries and notices alongside existing install/startup
checks. These checks remain separate from compositor and real-daemon proof.

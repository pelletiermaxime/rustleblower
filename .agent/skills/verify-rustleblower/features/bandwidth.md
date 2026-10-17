# Per-application data usage (development preview)

The user selected live rates and session totals. See
[the scope and measurement contract](../../../../docs/bandwidth-plan.md).

The native Data usage page shows per-executable rates and session totals,
search, global numeric sorting and 100-row pages. Ctrl+8 opens it; Start creates
an owned local receiver and launches the standalone Linux collector through
`pkexec --disable-internal-agent`. The desktop agent owns the password prompt;
Copy collector address is optional diagnostics. The GUI stays
unprivileged and there is no KDE dependency. Statistics counts connections;
Live sockets are not proof of transferred bytes.

## Current entry points

- `python scripts/bandwidth/build.py`: compile kernel-BTF-specific CO-RE probes and
  standalone libbpf loader into a new ignored evidence directory, then atomically
  stage that build at `target/bandwidth` for source UI discovery (`--no-stage`
  keeps isolated builds unstaged). No BPF loading or system installation.
- `python scripts/bandwidth/verify.py --workloads-only`: nine owned TCP/UDP/peek/
  truncation/exit scenarios inside a private user/network namespace, without BPF.
- `sudo /usr/bin/python -I scripts/bandwidth/verify.py --build /absolute/build`:
  root live-kernel verification in a private network namespace, targeting only
  its owned unprivileged workload PID/UID. Verify exact bytes, process identity,
  no loss/truncation, fresh samples, post-exit retention and owned cleanup.
- `sudo /usr/bin/python -I scripts/bandwidth/verify.py --short-lived --build /absolute/build`:
  three gated real curl processes, only their PIDs selected, exact per-process
  bytes and kernel-captured paths after every client exits before the first usage
  sample; repeat a second post-exit sample to prove retention.
- `python scripts/bandwidth/test_transport.py --build /absolute/build`: real C
  connector, same-user Unix stream and wrong-peer/path refusal, without BPF.
- `python scripts/verify-headless.py --feature bandwidth`: actual Kit navigation,
  start/copy/search/table header sorting/paging/stop, stream fixture, hidden-page
  collection, malformed-input retention, new-session totals and an independent
  permission RPC while a collector frame is incomplete.
- `python scripts/owned_net.py --evidence /absolute/network.json -- python scripts/cargo.py test --no-default-features --locked`:
  required core/integration suite, including its separately isolated HTTP fixtures.

Build and workload-only results cannot establish BPF verifier, attachment,
counter or overhead proof. A failed load must emit no zero-traffic snapshot.
Never change the daemon, active socket or host firewall. Keep root loading out of
the desktop process. Preserve logs and failed reports under `.verification/`.

## Remaining proof

The user has passed all nine live-kernel byte/attribution cases on the current
kernel. The updated path-capture collector also passes three short-lived curl
cases and the original nine byte cases (see evidence below). Broader concurrency,
shared-socket/container/QUIC/other-kernel-path/pressure and sustained overhead
evidence remains pending. Real collector-to-rendered-desktop, native/compositor,
packaged installation and actual desktop-agent authorization proof remain
separate. The source preview requests system authorization from Start; daily/monthly history and
destination detail are outside the selected first slice.

## October 17 prototype evidence

- Kernel `7.2.9-2-cachyos`: `.verification/bandwidth/build-20261017T185158742789Z/build.json`
  passes BTF signature checks and BPF/loader compilation with warnings treated as
  errors. GCC `-fanalyzer` also passes for the loader. Neither loads BPF.
- `.verification/bandwidth/run-20261017T185202039729Z/result.json`: nine workload-only
  cases pass in a private user/network namespace, with owned children reaped.
  `loaded_bpf` is false; this is not counter proof.
- The build directory's `unprivileged-load.json` records the actual unprivileged
  host invocation failing with exit 1 and zero output bytes. The associated log
  retains `EPERM`. No false zero-traffic snapshot is emitted.
- The user's `.verification/bandwidth/run-20261017T190739530621Z/result.json`
  establishes successful load/attachment/initial output and owned-process cleanup.
  It failed before the first case passed. That older harness retained neither
  later snapshots nor worker stderr, so the underlying measurement failure is
  unresolved. The failed evidence remains preserved.
- Diagnostic build `.verification/bandwidth/build-20261017T191104148441Z/build.json`
  passes BTF checks and BPF/loader compilation; loader static analysis passes.
  `scripts/bandwidth/test_verify.py` passes eight regressions for continuous zero
  snapshots, missing output, partial progress, freshness, overcount, loss,
  post-exit totals and labelled worker timeouts. These are harness proof, not a
  successful kernel byte-measurement result. Sudo still needs the user's password.
- The next user run, `.verification/bandwidth/run-20261017T191522125196Z/result.json`,
  completed the TCP4 workload and observed exactly 131,084 received bytes but zero
  sent bytes. All 17 matched hook entries had returns and accounted calls, with
  no lost operations. The run failed its byte assertion and cleaned up.
- The installed kernel's `.verification/bandwidth/inet-sendmsg-disassembly.txt`
  and `inet6-sendmsg-disassembly.txt` show direct calls to `tcp_sendmsg_locked`,
  bypassing the wrapper hook. The corrected collector hooks that implementation
  once. This diagnosed the missing path before the successful run below.
- `.verification/bandwidth/build-20261017T192306747992Z/build.json` passes the
  corrected BPF/loader build and real userspace executable-path checks. The path
  guard now compares statx mount ID/inode instead of equating a Btrfs subvolume
  stat device to the inode superblock device; wrong mount/inode/lifetime still
  fail. Ten Python diagnostic regressions and loader static analysis pass.
  `.verification/bandwidth/run-20261017T192227540389Z/result.json` retains another
  nine-case workload-only pass. Byte-count and path assertions remain strict;
  independent byte cases can finish before a path-only failure is reported.
- `.verification/bandwidth/run-20261017T193120934383Z/result.json`: all nine
  live-kernel byte and executable-attribution cases pass. Totals are 529,203 sent
  and 528,060 received bytes after all owned workloads, with no loss/truncation,
  post-exit retention and both processes exiting 0. The private namespace and
  owned cleanup are recorded. This replaces the earlier pending-counter status.
- `.verification/headless/20261017T200044Z-09f4c772/result.json`: ten-checkpoint
  usage Kit journey passes, including a real Deny-once RPC while a partial
  collector frame is held; general and window-state regressions also pass at
  `.verification/headless/20261017T195024Z-3a0b82fe/result.json` and
  `.verification/headless/20261017T195039Z-c83a780d/result.json`.
- `.verification/bandwidth/build-20261017T194635521752Z/transport-tests.json`:
  three actual C connector checks pass without BPF. The model and socket-worker
  tests cover monotonic rates, atomic refusals, PID/inode reuse, global paging,
  partial/lost data, fragmentation, stale/disconnected rates, restart and
  preserving a replacement file at the old endpoint name.
- `.verification/bandwidth/ui/final-bandwidth-core.log`: all nine focused
  model/socket tests pass, including unavailable rates across incomplete samples.
  `final-clippy.log` passes all-target/all-feature warnings-denied checks.
  `final-desktop-build.log` passes the actual desktop build. The language
  inventory passes for all five catalogues.
- The broad core/integration suite has one failing check:
  `published_disk_budget_rejects_candidate_and_preserves_last_good_file` in
  `tests/subscriptions.rs` times out. Its isolated retry and an unchanged
  `git archive HEAD` baseline at `387796e78f145498ca0930ccd6d4b6add50d4899`
  both reproduce that timeout. Evidence: `.verification/bandwidth/ui/` files
  `all-core-tests-isolated.log`, `subscription-recheck.log`,
  `baseline-source.json` and `baseline-subscription.log`.
  Remaining targets and the full library pass in `final-core-tests.log`.
  This is a pre-existing failing gate; no subscription timeout or logic was changed.


## UI authorization follow-up

Start resolves the helper off the UI thread and launches the absolute collector
through `/usr/bin/pkexec --disable-internal-agent`, with direct arguments and
null stdin. No shell, GUI elevation, password handling, authorization rule or
service is added. Only the current UID is selected; the helper lasts at most
one hour. Startup expires after two minutes. Four KiB of stderr is retained,
with at most sixteen KiB drained per worker tick. Cancellation, authorization
failure, collector failure and normal completion are distinct. Stop removes
the receiver before reaping the child, cancelling pending authorization or
causing an elevated collector to exit on the closed connection.

The twelve-checkpoint Kit run
`.verification/headless/20261017T201110Z-6f6a5298/result.json` passes real
Start/Stop input, cancellation, retry and Stop during authorization, in addition
to the original usage/permission checks. Only the external authorization program
is replaced by an owned child fixture; no real desktop prompt is claimed.
`.verification/bandwidth/authorization/core.log` passes all twelve focused model,
worker and launcher tests, including actual child-to-receiver measurements,
exit statuses 0/126/127/1, missing launcher, bounded diagnostics and child reaping.
`clippy.log` passes all-target/all-feature warnings-denied checks and `i18n.json`
passes all five catalogues. `build.log` identifies the staged helper build.
`library-isolated.log` passes the full library suite in its required owned network
namespace: 100 passed, one ignored. `desktop-build.log` passes the desktop build.
The earlier failed Kit report remains preserved: new catalogue entries were
initially placed outside the messages object; the corrected catalogues pass.
Actual desktop authentication, root collector-to-rendered-page startup and
packaged helper installation remain separate proof.


## Short-lived executable attribution

Paths are now captured at socket-operation time by two observational BPF LSM
programs, using a referenced executable file and `bpf_path_d_path`. Their return
value preserves the prior LSM result; they neither allow denied operations nor
reject successful ones. The process key includes execution ID, so replacing an
executable cannot inherit older cached metadata. An immutable 4,096-entry path
map and fixed per-CPU scratch buffers keep allocation bounded. The collector
uses captured paths directly, without querying `/proc` after processes exit.
Unknown paths are still never merged by short process name.

- `.verification/bandwidth/build-20261017T215147277574Z/build.json`: final fourteen-program
  BPF/collector build, required BTF signatures/kfuncs and userspace path validation.
- `.verification/bandwidth/run-20261017T215211992218Z/result.json`: all nine existing
  TCP/UDP byte/attribution cases pass with the final build, including retained
  kernel paths after workload exit.
- `.verification/bandwidth/run-20261017T215213348499Z/result.json`: all three real
  curl processes exit before the first usage sample, with exact sent/received
  counters and `/usr/bin/curl` paths sourced from the kernel. A second sample
  retains the same counters. The namespace, selected PIDs and owned cleanup
  are recorded; the system daemon was not changed.
- `.verification/headless/20261017T215018Z-b942ea2c/result.json`: thirteen Kit
  checkpoints pass, including three completed curl identities combined into
  one row with 6,000 received / 600 sent bytes. Core coverage also checks an
  exec replacement with reused PID/lifetime/inode and a distinct path.
- `.verification/bandwidth/short-lived-library.log`: 101 library tests pass,
  one ignored, in the required private network namespace.
- `short-lived-clippy.log`, `short-lived-desktop-build.log` and
  `short-lived-analyzer.log` pass warnings-denied Rust checks, the actual desktop
  build and C static analysis. Ten Python verifier regressions pass.

The user's earlier desktop report and screenshot confirm Plasma authorization
and rendered live measurements with the previous collector after enabling
camera access for Howdy's polkit helper. The new attribution path has kernel
and Kit proof; no new compositor or package proof is implied. Historical build
and failed-run reports remain preserved. Already exited unresolved records from
an older collector session cannot be retroactively named; start a new session.

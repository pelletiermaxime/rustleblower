---
name: verify-rustleblower
description: Verify Rustleblower through the pinned GPUI Kit headless journeys and owned native desktop adapter. Keep protocol, persistence, interaction/layout, real pixel/compositor and package proof separate. Preserve evidence and scoped cleanup.
---

# Verify Rustleblower

Read [features/README.md](features/README.md) and the selected feature first.
The pinned daemon reference and protocol boundary remain under
[docs/upstream-reference.md](../../../docs/upstream-reference.md). Never change
the system daemon or replace an existing socket to run verification.

## Headless interaction and layout checks

The headless journeys use GPUI Kit's `test-support`, `TestAppContext`,
native event helpers and bounded condition waits. They use the standard test
text system, not a WGPU renderer. No headless screenshot, painted-color or
real-font proof is produced.

From the repository root:

```sh
python scripts/verify-headless.py --feature settings
python scripts/verify-headless.py --feature history-selection
python scripts/verify-headless.py --all
```

The default without `--feature` retains the broad `all` journey; `--all` runs the
complete feature inventory in isolated processes. List feature names with:

```sh
python scripts/headless_suites.py
```

Implementation and proof boundaries:
[docs/headless-testing.md](../../../docs/headless-testing.md).

Require a passed `result.json`, the intended feature and exact test name,
`framework: gpui-kit`, `real_gpu: false`, valid listed JSON checkpoints, owned
process exit and scoped socket cleanup. Read the feature's independent
state/protocol/persistence/output assertions; a checkpoint file by itself is
not feature proof. Compilation, execution and total wall time are separate.

The verifier removes display variables only in its child. Each journey owns
its socket/database and temporary fixtures. HTTP scenarios additionally require
a distinct mapped-user, loopback-only network namespace. Product actions must
use GPUI input. Fixture setup may enter through production backend APIs; never
set feature state to impersonate user input. Real expiry and process-boundary
checks retain their actual deadlines rather than substituting virtual time.

The suite is the background flow while the user works. Missing GPU hardware is
not relevant to this Kit test context; native and package checks still need
their own compatible display/renderer environment.

## Native desktop and package proof

Use the existing owned native adapter when the question involves real fonts,
window focus, keyboard/pointer delivery through the OS, placement or compositor
behavior:

```sh
python scripts/cargo.py build --locked
python scripts/verify-native.py doctor
python scripts/verify-native.py run --feature prompts
python scripts/verify-native.py run --feature history
python scripts/verify-native.py run --feature tray
```

Consult the feature map and adapter help for the focused native journeys. These
runs own their app PID, input driver, fixture daemon, database and socket. They
never adopt the user's existing app, take over `/tmp/osui.sock`, or change the
system daemon. The foreground driver takes FerryDock's advisory input lock;
do not run another foreground driver or manually drive that session at once.
Only remove owned resources, and retain evidence after failure.

On Plasma Wayland, use the native KWin adapter. An explicit X11 check may use
`--backend x11`; never silently substitute XWayland for failed Wayland proof.
Placement requests do not prove that Wayland applied a corner position. Check
the owned window's actual geometry and input result. Tray callback proof does
not establish a physical panel pointer click.

Run `scripts/verify-packages.py` for the packaged artifact's install/startup
boundary. Do not equate build or Kit test success with successful packaged
launch, real font rendering, OS IME input, daemon enforcement or pixel output.
The native adapter and packaging code retain their FerryDock MIT notice.

## Core checks and reporting

```sh
python scripts/cargo.py test --no-default-features --locked
python scripts/cargo.py clippy --all-targets --all-features --locked -- -D warnings
```

Protocol/storage workers must remain bounded and off the UI thread. Permission
handling must not wait for telemetry ingestion, SQLite or main-window visibility.
Do not remove those assertions to shorten a test.

Report the exact commands/revision, passed and failed journeys, evidence paths,
owned cleanup and proof boundary. Preserve ignored `.verification/` directories,
including older GPU screenshots. Clean only processes, sockets and displays
owned by the current run. A failed check is a failure even if other journeys pass.

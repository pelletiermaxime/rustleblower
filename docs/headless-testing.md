# Headless UI testing

All 42 headless journeys use the pinned GPUI Kit testing framework. Tests live
under `src/app/kit_tests/`, one journey per feature module, with shared context,
input, layout and condition-wait helpers in `src/app/kit_tests.rs`.

Run a journey from the repository root:

```sh
python scripts/verify-headless.py --feature history-selection
python scripts/verify-headless.py --feature settings
```

Run the complete suite, with each feature in its own process:

```sh
python scripts/verify-headless.py --all
```

`scripts/headless_suites.py` owns the feature-to-test inventory and four isolated
CI batches. Its coverage guard requires every registered module to use the Kit
test runner, with no missing, duplicate or unregistered journeys. CI compiles the
headless executable separately from running the journeys, so build time and
execution time remain distinguishable.

## Input and synchronization

`#[gpui_kit::test]` supplies the GPUI test context and executor. Tests open the
production views and dispatch actual GPUI pointer, key and text events through
Kit's `TestWindowExt`. Custom bounded wheel/Shift-click helpers still dispatch
native GPUI events; they do not change application selection state directly.

The synchronous journeys share a small adapter around Kit's asynchronous
`TestAppContextExt::wait_for`. The captured History selection journey uses the
async runner directly. Both use the same Kit API and test clock. For example:

```rust
click(&mut cx, main, "older");
wait_for(&mut cx, |cx| {
    cx.read_entity(&view, |view, _| view.history.page.rows.len()) == 5
});
```

Frame processing advances simulated time and completes layout without sleeping
for 400 ms. Shared input waits for its target to exist and be visible. Data reads
use test-only, read-only readiness observations for the active page; command
acknowledgements are kept separate so deliberately held ACKs do not prevent
independent permission tests. Page-readiness waits also yield when a permission
is pending, so permission input never waits for an unrelated page read.
Permission-window waits match the window to the
current backend request rather than assuming a prompt has already appeared.

Conditions have a bounded Kit-clock timeout and a 15-second real deadline. The
verifier adds a process deadline and cleans up its owned child process group.
Some journeys intentionally wait for actual request expiry or subprocess exit;
those checks retain real elapsed time. Advancing a simulated clock is not proof
that a real daemon/storage/network deadline elapsed.

The fixtures intentionally use real SQLite and Tokio/gRPC worker threads. Test
setup permits external worker wakeups, then restores advancing UI test time.
This does not claim deterministic scheduling of external I/O. HTTP journeys use
an owned mapped-user, loopback-only network namespace through `owned_net.py`.
Sockets and databases are disposable and owned; `/tmp/osui.sock` is never used.

## What a passing headless run proves

Each evidence directory contains:

- `test.log` and `result.json`, with separate build/test/wall times.
- `headless.json`, identifying GPUI Kit, `real_gpu: false` and socket cleanup.
- JSON interaction/layout checkpoints and feature-specific proof reports.
- Actual fixture output where relevant: exported files, wire commands,
  independently reopened persisted settings, or exact clipboard comparisons.

A layout checkpoint records window metrics and a bounded registry of observed
controls. It is not an image. The verifier rejects a report claiming real GPU
proof, missing JSON checkpoints, invalid checkpoint JSON or missing cleanup.
The tests' behavioral assertions establish the relevant results; checkpoint
existence alone does not prove a feature is correct.

These tests use GPUI's standard test text system. They no longer use the WGPU
headless renderer, capture screenshots, compare painted colors, or establish
real-font/pixel correctness. Permission-icon tests check lookup/loading/fallback
state; highlighting checks configured style state and selection suppression.
Native compositor, OS input/focus/IME, real font rendering and packaged startup
remain separate verification boundaries. Use the owned native adapter and
package verifier for those checks. Their screenshot/OCR behavior is unchanged.

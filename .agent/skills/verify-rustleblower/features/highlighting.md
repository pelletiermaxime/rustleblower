# Configured highlighting

Read [source, model, bounds and integration contract](../../../../docs/highlighting-plan.md). The bounded background loader and compiled literal cell/row matching are implemented. Original source numeric columns require explicit stable native-key maps. Selected rows keep native selection colors; arbitrary regex/fonts are source TODOs.

Focused command: `python scripts/cargo.py test --no-default-features --locked --offline --lib highlighting::tests`. Four pure/file/worker cases pass in `.verification/highlighting-20261016/core.log`. Backend lifecycle and native render adapters are implemented; actual GPU/input proof remains separate from the independent model tests.


The dashboard **Reload table styles** action (`highlighting-reload`) reads the
bounded native `<database>.highlighting.json` through its backend worker. There
is no automatic file watcher. Native History cards/Kit table, Rules, Nodes, all
six statistics kinds, Alerts, Firewall and LiveSockets use the explicit column
keys documented in the integration contract. Human display text follows the
active locale; daemon/user data and action/duration values stay unchanged.
Selected rows bypass configured styles. An invalid Reload retains the last good
profile and reports its warning without delaying a permission reply.

`cargo test --offline --locked --all-features --lib app::highlighting::tests::history_cache_tracks_page_style_generations_selection_and_bound -- --exact`
passes in `.verification/highlighting-20261016/native-cache-test3.log`. The
History delegate cache is capped 64 rows and keyed to page/style Arc generations;
the test covers same-id metadata replacement, same-revision style reload,
selection, capacity and clearing. This is a focused state/resource test, not
native painting proof.

The dedicated native journey is `python scripts/verify-headless.py --feature highlighting`. Nine actual background GPU/input frames pass in `.verification/headless/20261017T041912Z-a4566784/result.json`; scoped aggregate `.verification/highlighting-gpu-result.json` retains the137-file source manifest and immutable test binary. The actual PNG proof counts88,507 GREEN history-cell pixels,109,021 MAGENTA later-row pixels and zero overridden BLUE pixels. Selected History/Rules rows contain zero configured color pixels while another rule retains CYAN. The pinned Kit table renders23,337 GREEN and76,868 MAGENTA pixels; disabling profiles clears both tozero.

The run also verifies invalid-file last-good preservation/exact bytes, repair through actual Reload, column/table controls and an independent real isolated AskRule/Deny. Selection legitimately opens Rules actions and shifts rows, so the verifier samples the selected row's fresh observed bounds. Initial failed evidence is retained, including the production cached-node lookup panic fixed by replacing a Tokio-only flush on the GPUI executor with the existing bounded executor-independent queue fence. This focused rendered-color proof covers History cards/table and Rules. Other adapters retain core/build evidence; real daemon enforcement, native compositor/package behavior, RSS and frame latency are separate boundaries.

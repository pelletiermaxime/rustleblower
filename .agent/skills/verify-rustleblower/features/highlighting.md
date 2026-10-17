# Configured highlighting

Read [source, model, bounds and integration contract](../../../../docs/highlighting-plan.md). The bounded background loader and compiled literal cell/row matching are implemented. Original source numeric columns require explicit stable native-key maps. Selected rows keep native selection colors; arbitrary regex/fonts are source TODOs.

The dashboard **Reload table styles** action (`highlighting-reload`) reads the
bounded native `<database>.highlighting.json` through its backend worker. There
is no automatic file watcher. Native History cards/Kit table, Rules, Nodes, all
six statistics kinds, Alerts, Firewall and LiveSockets use the explicit column
keys documented in the integration contract. Human display text follows the
active locale; daemon/user data and action/duration values stay unchanged.
Selected rows bypass configured styles. An invalid Reload retains the last good
profile and reports its warning without delaying a permission reply.

## Verification

Run `python scripts/verify-headless.py --feature highlighting` for actual Reload,
History/table/Rules controls, style precedence, selected-row style suppression,
invalid-file preservation, exact file bytes and an independent AskRule/Deny.
The assertions inspect configured style state; native painting requires a native
run.

Core loader/model tests use `python scripts/cargo.py test --no-default-features --locked --lib highlighting::tests`.
The `app::highlighting::tests` desktop unit tests check the 64-row cache,
page/style generations, metadata replacement, selection and cache clearing.

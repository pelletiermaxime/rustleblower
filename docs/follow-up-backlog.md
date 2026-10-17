# Follow-up backlog after the daily-use checkpoint

On 2026-10-17 the user narrowed the finish line to a verified daily-use app.
Exhaustive OpenSnitch UI parity and additional ideas are follow-up work, not
requirements for this checkpoint. This document records the deferred scope;
it does not schedule or authorize a new implementation run.

## Daily-use checkpoint

Permission replies, manual application-rule management after missed prompts,
disk-backed history, connection/node state, statistics, alerts and Live socket
inspection are implemented. The latest compact shell and History/Rules cleanup
passes the existing general, History-selection, advanced-Rules and window/menu
GPU/input journeys. See `visual-cleanup.md`, the verification skill feature map
and `.verification/compact-shell-result.json` for exact source and proof scope.
The rich Live table has its separate final ten-frame report.

## Deferred parity and presentation

- Grouped retained-history detail windows and direct category/node/rule entry
  points. The private snapshot/output core is preserved and tested; no grouped
  detail UI is linked. See `grouped-history-plan.md`.
- Additional native table adapters, global sorting, saved columns/widths and
  selected output where useful in Nodes, Rules and Alerts. Existing functional
  views remain usable. Shared presentation/core work is preserved; see
  `view-presentation-plan.md` and `completion-checklist.md`.
- Remaining translations, native CJK/RTL verification and further localization
  polish. English/French and partially translated additional catalogues retain
  explicit coverage and fallback boundaries; see `localization.md`.
- Remaining reputation/process/host entry-point equivalence, local display-name
  lookup and smaller source-backed parity details in `completion-checklist.md`.
- Further toolbar/form polish and complete light/dark/minimum-size review beyond
  the scoped shell/History/Rules batch.

## Later release and hardening work

Fresh packaged-install/startup and compositor checks, current real-daemon
end-to-end confirmation, and sustained RSS/disk/latency measurements remain
separate evidence layers. The background fixtures do not claim those results.
No verification should replace the real daemon socket or change host policy.

OpenSnitch issue/PR ideas and Little Snitch-inspired additions can be assessed
after a separate prioritization decision. They are not part of the daily-use
finish line. Published packages retain their existing version until a separately
authorized release is built and published.

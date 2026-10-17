# Full parity reference and follow-up checklist

The 2026-10-17 finish line was narrowed by the user to the verified daily-use
app. This exhaustive source audit is a follow-up reference, not a list of blockers
for that checkpoint. See [follow-up backlog](follow-up-backlog.md).

Audit checkpoint: 2026-10-16, current worktree after v0.2.3 (`d0c57cf`),
including current uncommitted parity work. The complete core checkpoint passes263
cases (`.verification/whole-core-retry-result.json`); newer native changes have
separate compile and immutable GPU reports.
This is a source audit, not a new test report. Read the exact feature maps for
existing evidence; a missing native check does not mean the feature is absent.
Concurrent implementation can supersede a row below before the next checkpoint.

The authoritative OpenSnitch reference is the retained bare Git repository at
`/home/maximep/.cache/paru/clone/opensnitch-git/opensnitch`, commit
`a1353848ba1b660320e90cefea782c3fba272c00` (source 1.9.0). Upstream paths below
are relative to that pin, not a moving branch. Read with `git show`; do not change
the package cache. Rustleblower and FerryDock both pin GPUI Kit 0.7.3 with the
single GPUI Fast 0.1.7 backend. Preserve that architecture and keep I/O off the
UI/prompt path.

## Source-backed parity work and current status

| Work item | Exact source contract | Current source and bounded implementation task |
| --- | --- | --- |
| Missed permission popup notices | `ui/opensnitch/config.py:133-138`, `service.py:637-688`: editable message template with connection/rule/node fields, notification after an unanswered popup. | Implemented with four dedicated core cases, four preference regressions, private-D-Bus routing and a 16-frame GPU journey. Uses the actual captured fallback, current notice preferences, bounded single-pass templates and Application rules action. See [contract and proof](missed-popup-notifications.md); painted desktop banners/physical clicks remain separate. |
| Firewall service/interface suggestions | `ui/opensnitch/dialogs/firewall_rule/statements.py:60-84,398-402`, `dialog.py:514-534`: local network-service name/port mapping and local interface suggestions. | All 17 typed statement constructors and guided verdict hook/parameter controls now exist; see the implemented checkpoint below. Bounded background service/interface suggestions and reload now exist, with two owned-fixture core cases and a 22-frame GPU journey proving alias ambiguity, service protocol/port fill, interface selection and reload preservation (`.verification/firewall-suggestions-result.json`). Numeric/range/list and manual interface/wildcard input remain available. Native compositor and unusual raw-only variants remain separately scoped. |
| Advanced history expression equivalence | `ui/opensnitch/dialogs/events/queries.py:16-30,35-73,145-200`: cwd, text/inclusive operators and compound expressions. | Cwd, not-contains/starts/ends, inclusive operators and bounded two-level AND/OR groups now have production code and focused core proof (`docs/advanced-history-search.md`). A newer shared bounded formula now represents nested AND/OR directly (eight conditions, eight parenthesis levels, 256 bytes, fifteen nodes); two parser and ten affected integration cases pass, with nine history/ten rule GPU frames passing (`.verification/search-formula-result.json`). The seven-frame GPU/input journey passes (`.verification/history-advanced-result.json`); arbitrary SQL/functions and unbounded expressions remain outside the native contract. |
| Advanced rule catalogue search | `ui/opensnitch/dialogs/events/queries.py:75-105,170-174`: created/time, name/description/node, enabled/action/nolog/priority/duration and operand type/name/data filters. | Typed field/operator and bounded two-level group filters now apply across catalogue paging, selected/full-filter operations and exports. Eighteen focused integration cases, a grouped snapshot unit case and the eight-frame GPU journey pass (`.verification/rules-advanced-result.json`). New bounded nested AND/OR formula support has parser/core and nine-history/ten-rule GPU proof; local receipt-time predicates and explicit nested operand/data selectors now have31 affected core cases passing, including schema9 migration with unknown receipt and byte-preserved wire documents. Their fourteen-frame native GPU/input journey passes, including actual receipt and nested selectors and byte-preserved exported documents; see [scope](advanced-rule-search.md). |
| Cross-page table selection and selected-row exports | `ui/opensnitch/customwidgets/generictableview.py:283-307,343-411,457-620`, `dialogs/events/views.py:231-260,380-411`, `menu_actions.py:36-72`: database-backed multi/range/select-all and selected row copy, separate full CSV export. | History now has a bounded captured query/range/mark descriptor and chosen-column selected CSV/JSON Lines/file/clipboard paths; seven focused core/export cases and nine actual GPU/input frames pass ([contract](history-selection.md)). Statistics captured-snapshot marks/ranges/all and selected-column outputs are linked with three focused core cases and four protocol regressions passing; concrete native and strict core checks pass, plus fourteen actual GPU/input frames ([contract](statistics-selection.md)). Rule marks already cross pages. Live selection/presentation is being implemented independently; Alerts and remaining relevant table surfaces still need selected output semantics. |
| Other-page presentation persistence | `ui/opensnitch/config.py:144-164`, `dialogs/events/views.py::restore_details_view_columns`: per-view header/column state, hidden tabs, splitters and live socket filters. | History widths/search/conditions/sort and main geometry/page restore exist. Nodes, rules, alerts, live and category/detail views lack equivalent saved presentation controls. A finite shared `.views.json` core model/worker and optional Rules global-sort reader are published and core-tested, including8detail profiles, but the three main Nodes/Rules/Alerts table adapters and saved-view controls have not been started. Existing cards remain. See [explicit checkpoint](view-presentation-plan.md). Qt platform settings are not port requirements. |
| Theme catalogue equivalence | `ui/opensnitch/dialogs/preferences/sections/ui.py:21-58`, `utils/themes/themes.py:34-103`: discovered installed themes and saved theme selection. | Native density and custom palette controls were added concurrently after the initial source pass; see `docs/density-plan.md` and its proof boundaries. Assess whether saved native palette choices close the useful catalogue/customization behavior. Qt XML execution is unnecessary; system-change callbacks and final layout proof remain separate. |
| Language selection and translations | `ui/opensnitch/dialogs/preferences/sections/ui.py:8-18,74-83`, `settings.py:180-183`, `utils/languages.py:12-40`, `ui/i18n/locales/`: saved language/system locale and shipped translations. | The bounded locale model, saved System/explicit choice, original EN/FR catalogues and actual pinned Kit translation backend now exist. Eight GPU frames prove save/restart-required/fresh-context French activation, typed French filters with exact rows and missing-language fallback. Broad call-site/template migration and the remaining source locales/RTL/CJK remain active; see [scope](localization.md). |
| Configurable view highlighting | `ui/opensnitch/plugins/highlight/highlight.py:12-79,133-137,179-295`, `actions/default_configs.py`: file-configured row/cell text matching, text/background colors and alignment; common/rules/firewall/netstat defaults. | Native semantic colors belong in the final consistency phase. A bounded file-configured native model, explicit source-index import mapping, compiled ordered styles and off-thread last-good loader now exist; four focused pure/file/worker cases and strict core Clippy pass (`.verification/highlighting-20261016/result.json`). App lifecycle, native column maps/style application, immutable Arc snapshots and per-row prepared matching/cache adapters are linked; concrete desktop and focused cache checks pass. The nine-frame native highlighting journey is running from an immutable snapshot. Upstream `configure()` is TODO; a graphical rule editor is optional. |
| VirusTotal native/prompt integration proof | `ui/opensnitch/plugins/virustotal/virustotal.py:147-225,294-346,348-414,437-555`, `_models.py:147-240`, `_procdialog.py:35-68`: optional domain/IP/hash checks, exclusions/scoring, hosts/process details and popup warning/default-action effect. | The native model/store/worker and combined Kit settings/report singleton now exist, with seven isolated worker cases passing and included in the combined 238-case core run. The 15-frame GPU journey passes (`.verification/reputation-result.json`), covering actual settings/results controls, early/late warnings, untouched recommendation, capture isolation and held-HTTP permission independence. Root-owned backend lifecycle/protected-path integration is present; the popup scroll repair and fixed decision footer now also pass a fresh15-frame journey (`.verification/headless/20261017T034508Z-037b159a/result.json`). Credentials use an owned key file, endpoints are fixed, and live effects require current capture/revision/untouched UI; no remote wait may gate permissions. Hosts-table/process entry-point equivalence remains a separate source surface until hooked/proven. See `docs/reputation-plan.md`; source TODO malicious desktop notices/file uploads are not parity requirements. |
| Legacy URL-to-file download action | `ui/opensnitch/plugins/downloader/downloader.py:23-43,61-104,123-169,188-217`: configurable scheduled enabled URLs/local filenames with aggregate success/error notices. | This is distinct from both daemon Live downloader tasks and the newer normalized list-subscription manager. Bounded raw-file worker, source metadata-preserving profile interchange, reviewed native singleton configuration and manual-run view are implemented; eight isolated core/file/HTTP cases pass (`.verification/raw-downloads-20261016/result.json`). Application integration8/private D-Bus1 and raw12 real-GPU/input proof now pass (`.verification/raw-downloads-result.json`). Final worker8 rechecks two post-GPU safety deltas: stage-directory sync before journal cleanup and protection of the new highlighting document. No external download/package/compositor proof is implied. No source GUI is missing (`configure()` is empty). |
| Temporary catalogue policy/status | `ui/opensnitch/config.py:246-267`, `service.py:150-159,884-895`, `rules.py::disable_expired`: optional omission of short-lived cached rules and expiry of displayed temporary entries. | The source-ordered threshold, schema11 process-random observation markers, separate daemon-enabled/cache-status presentation, frozen query/export policy and localized settings/review singleton are linked. Offline cached-node selection is reachable through a bounded searchable control. Final four model/worker cases and three integration cases pass, including byte-preserving schema10 migration, real RPC observations and frozen650rule×2node query/export scopes; initial strict core Clippy and concrete linked headless checks pass. Genuine two-process native input/restart proof passes (six frames,15.92s; `.verification/headless/20261017T042151Z-476fe547`). Exact core hashes/logs are in `.verification/catalogue-policy-20261016/`. No daemon timer or policy mutation is introduced. |

## Category/detail matrix to close explicitly

The category list itself is present: `src/statistics.rs:24-68` covers applications,
hosts, addresses, ports, users and protocols with matching-history drilldown.
Upstream categories are `dialogs/events/constants.py:72-85`; detail grouping is
`queries.py:511-594`. Do not call these categories absent merely because native
navigation differs.

| Surface | Existing implementation | Remaining equivalence/proof decision |
| --- | --- | --- |
| Events | Indexed history, details, matching rule/new rule, full-match sort/export, saved filters and widths. | Advanced operators/cwd, multi-selection and selected exports above. |
| Applications/processes, hosts, addresses, ports, users | Daemon counters, global category sort, frozen pages, CSV/JSON/clipboard, exact retained-history drilldown and confirmed reset. | Compare upstream grouped detail hit counts and category/detail columns/export. Retained event rows and daemon counters have different provenance; label it. UID/name and service-name lookup should be checked against source display behavior. |
| Nodes and matched rules | Node detail, selected-node rules, history node/rule typed conditions, process inspection. | Expose direct node/rule grouped-history navigation if equivalent information currently requires manual filter construction; verify multi-node ties and offline cache behavior. |
| Live sockets | Filtered latest bounded socket snapshots with process inspection. | The thirteen-column Kit table, global typed sorts, captured cross-page/range/all selection, frozen CSV/JSON output and saved protocol/family/state/search/sort/column presentation are linked. Four focused core cases and concrete/strict native compilation pass. Eleven legacy GPU/input frames and ten richer table frames pass on their retained source checkpoints. See [scope](live-table-plan.md). |
| Rules, alerts, firewall tables | Functional browse/edit/review/export paths and bounded pages. | Final consistency plan must add visible sorting wherever meaningful; rules currently order only by name (`src/rules.rs:158-163`), nodes only by name (`src/nodes.rs:211-214`). Preserve ordered firewall rules as semantic order rather than sorting their mutation order accidentally. |

## Implemented scope needing targeted verification or recovery work

- [ ] Release checking now implements the pinned versionchecker schedule/notice
  intent (`plugins/versionchecker/versionchecker.py:39-57,76-103,129-159`). The
  independent worker/native settings view and root-owned lifecycle, selected-daemon
  watch, bounded notice/action routing and protected preference path are present.
  Final nine focused owned-HTTP/core cases pass after the lazy-client/bounded-
  shutdown/view-opening refinements. The 11-frame GPU journey, selected-daemon
  integration and private-D-Bus routing pass (`.verification/updates-result.json`),
  including manual checks while automatic checks are off, reopening, cancellation,
  stale-save refusal and independent permission input during held HTTP. Automatic
  checks default off; explicit manual checks use saved settings. No installation
  or unrelated-version comparison occurs. Read [scope](update-checker-plan.md).
- [ ] Remote TCP/TLS/mTLS transport now has production code and six actual owned
  loopback cases: positive/refusal TLS contracts, trusted peer identity/locality,
  held-ACK permission/Ping independence, reconnects, bounded initial deadlines,
  nonwaiting method admission and startup arguments. Default Unix ownership
  behavior is retained. Full core passed 200/0 with stable hashes, before the
  narrow IPv6 scope-key hardening; a final focused rerun passes all six transport
  cases plus five helper cases, with source hashes in
  `.verification/remote-transport-20261016/final-focused-result.json`. Exact Go-client
  proof is unavailable offline (grpc-go v1.32 dependencies are uncached; no fetch
  attempted). Real remote host/package/RSS proof is not claimed. Read
  [transport scope and memory bounds](remote-transport.md), including link-local
  conservative locality and supported endpoint forms.
- [x] Subscription result notices have production code, focused four-case core
  proof, private-D-Bus delivery/routing proof and the current nine-frame GPU
  preference/review/hidden-window journey. Check exact retained reports for the
  imported text, cancellation/304/publication-failure and failure-delivery bounds. This
  closes a distinct source contract from daemon alerts and missed-popup notices
  (`list_subscriptions.py:324-339,1056-1066`).
- [ ] Firewall helpers now construct all 17 statement families and guided verdicts
  using Kit controls, preserving the complete raw editor and existing review/ACK
  path. Five source-fixture/boundary/roundtrip core cases pass; desktop compilation
  passed. The expanded 18-frame GPU/input/protocol/cache journey passes after fixing
  Select-host hit bounds with constrained trigger rows. Native compositor helper
  input and all option combinations are not yet claimed. See
  [helper scope and exact limits](firewall-helpers-plan.md); service/interface
  suggestions and unusual raw-only numeric/verdict variants are explicit.
- [ ] Native density and custom palette controls now have production code and
  eight focused core cases and desktop-check evidence from the owning agent.
  The combined 24-frame density/palette GPU journey passes
  (`.verification/density-palette-result.json`). Final overflow/contrast proof belongs to the whole-app phase; do not
  describe System/Light/Dark alone as the current capability.
- [x] Bulk-unit/default-review verification passes: 61 subscription, 12 inheritance
  and 48 general GPU frames, with focused core checks and frozen-default stale
  guards (`.verification/subscription-bulk-units-result.json`). Native new unit
  gestures and final resource/package checks remain distinct.
- [ ] Publication now has a durable one-row intent, retained hard link, atomic
  SQLite metadata/log/intent completion, idempotent rollback and a durable
  obsolete-filename cleanup queue after collection synchronization. Five fault/
  reopen cases, 42 impacted integration cases and the current 61-frame
  subscriptions GPU regression pass with matching journal source hashes.
  Physical power-loss and the complete disk/RSS workload are not claimed. Read [recovery contract](subscription-publication-recovery.md).
- [ ] Exercise retained old-format files and the physical disk budget under
  repeated URL/format/filename changes. Current publication lookup/UI focuses on
  current kind, while old per-kind data intentionally supports existing rules.
  Never delete referenced data or infer that SQLite byte totals include orphaned
  files. Inspection/cleanup controls are an optional native improvement, below.
- [ ] Obtain focused owned compositor evidence for remaining controls where
  placement/lifecycle/native input matters: range editing, cross-node review,
  native file pickers, rule/statement/value forms, successful URL probe/cancel,
  default/stale/reopen and density/appearance variants. Existing background GPU
  journeys remain evidence for the paths they actually drive.
- [ ] Validate real system-appearance callbacks and process/package startup with
  the final changes. Earlier fixed-Light GPU proof does not prove a desktop
  appearance callback, a current AppImage or a login autostart scheduler.
- [ ] Run bounded concurrent workload/RSS/disk/latency measurements and package
  checks for the final implementation. Keep core, rendered/input, protocol,
  persistence, packaging and real-daemon/enforcement evidence separate. No real
  daemon/socket change is necessary for owned verification.

## Independent task ownership and sequencing

| Task | Files that can be owned independently | Shared integration/conflicts |
| --- | --- | --- |
| Subscription notices — active root | Subscription model/runtime + notification mailbox/worker | Root reserves `src/backend.rs`, `src/app.rs`, subscription files and notification modules. |
| Native density — active agent | `src/appearance.rs`, `src/app/appearance.rs`, `tests/appearance.rs` | Custom themes/language must coordinate rather than concurrently edit appearance schema. |
| Firewall helpers — proposed next independent slice | New `src/firewall/helpers.rs`, new `src/app/firewall_view/helpers.rs`, existing `src/app/firewall_view/forms.rs`; dedicated core tests. | Tiny `src/firewall/mod.rs` module hook; leave backend and common headless harness to root/verifier. Full helper contract is the 17 categories and verdict constraints above. |
| Search/query parity | `src/history/query.rs`, metadata/storage migration and history tools; then rule query/document fields | Conflicts with final table pass and root-shared app composition; split history and rule queries only after agreeing the shared query model. |
| Remote transport — implemented, focused six-case final hardening proof passed | `src/transport.rs`, `tests/remote_transport.rs`, configuration/startup integration | Backend ownership returns to root for missed-popup notices. Preserve local-only subscription guards; network tests require the owned namespace. |
| Integrations/version checking | Separate bounded config/worker/result modules per integration | Common notification/action dispatch and prompt hooks require a small reviewed root integration. File-only action support does not require a generic plugin ABI. |
| Verification — active agent | Common headless harness, scripts and proof reports | One owner for `src/app/kit_tests.rs` and its feature modules and shared scripts; implementations provide small specific hooks/fixtures rather than competing edits. |

Complete independently owned functionality first; then reconcile shared
navigation/action surfaces and rerun impacted journeys. Do not claim the full
goal complete after only the currently active notification/density/bulk slices.
The missing locale, transport, integration and query work are material features.

## Native improvements and final required phase

These are not source-parity blockers: a metadata-archive management page/manual
resolver, an old-format retained-file browser/garbage collector, a per-target
operand difference inspector, a graphical highlight-rule designer, Python plugin
execution, Qt platform/scaling controls, or new per-application bandwidth work.
Keep recovery records and reimport capability while these optional tools are
absent. Resource correctness and last-good data preservation remain requirements.

- [ ] After parity, research current OpenSnitch issues/PRs and official Little
  Snitch capabilities with primary-source links; choose suitable native additions
  explicitly rather than copying every idea or source TODO.
- [ ] Execute the required [whole-app UI/UX consistency phase](ui-consistency-plan.md)
  after functionality/additions: shared layouts, meaningful global header sorts,
  semantic colors, editing/back/cancel/context retention, keyboard behavior,
  light/dark/density and target-font rendered/input evidence across the screen
  matrix. One polished screen or a green build does not close this phase.

The initial audit ran no test, build, foreground session, live network request or
daemon command. Later implementation/proof checkpoints above identify their own
scope. No cache, system daemon or existing socket was changed.

Final catalogue native/core aggregate: `.verification/catalogue-policy-gpu-result.json` (native6 plus core7; fifteen overlapping final source hashes match the immutable141-file snapshot). No real daemon, package or compositor claim.

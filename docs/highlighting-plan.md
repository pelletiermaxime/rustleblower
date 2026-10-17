# Configured table highlighting

The source contract is pinned OpenSnitch `a1353848ba1b660320e90cefea782c3fba272c00`, `ui/opensnitch/plugins/highlight/highlight.py:12–79,142–177,179–345` and `actions/default_configs.py`. Cell rules use exact text membership on selected columns. Row rules use substring OR across the configured columns/texts. Cells run first, then rows, in file order; later matching provided properties win. A later alignment replaces the earlier alignment. Selected rows keep the platform selection colors. Font and operator support are TODO, and `configure()` has no graphical editor. A regex/font engine or graphical rule designer is not a source parity requirement.

`src/highlighting.rs` supplies the independent original native model and compiled in-memory matching. It uses stable native column keys, rather than assuming Qt column numbers match Rust tables. `import_source(json, view, source_index_to_native_key)` accepts the original `actions.highlight` envelope or the highlight object and requires an explicit map: unmapped indices refuse import. Unknown source metadata/operators/font fields are not executed; the original source document is never changed. Native selected-state colors override configured styles. Source margin adjustments become one bounded final horizontal/vertical padding pair, avoiding cumulative Qt rectangle mutation.

Color parsing uses already pinned/cached MIT/Apache `svgtypes=0.16.1` for SVG names and CSS color forms, with explicit QColor hexadecimal handling: eight digits mean `#AARRGGBB`, not CSS alpha last, and nine/twelve-digit RGB values are scaled to bytes. No Qt/GPL implementation code is copied.

The native document appends `.highlighting.json` to the complete database filename. It has `version: 1` and `settings.profiles`, with at most 16 fixed view kinds. Each profile has `view`, `enabled`, `cells`, and `rows`. Rules contain native `cols` string keys, `text` strings and optional `color`, `bgcolor`, `alignment`, `margins` properties. Example:

```json
{"version":1,"settings":{"profiles":[{"view":"rules","enabled":true,"cells":[{"cols":["action"],"text":["reject"],"color":"purple"}],"rows":[]}]}}
```

The complete document is at most 64KiB; each view has at most 64 rules, each rule1–16 columns/texts, column keys1–32 ASCII letters/digits/underscore/dash and text1–128 bytes without controls. Colors are1–64 bytes, alignment accepts only center/hcenter/vcenter, and margins are0–32. A rendered row supplies at most 64 displayed cells and at most 16KiB per value; oversized inputs receive no custom style. Matching is literal, synchronous, memory-only and makes no storage/network calls.

The loader has one background thread, a four-slot refusing command queue, no periodic watcher/timer/network, and an atomic stop flag. It loads at startup or explicit Reload. Invalid, oversized, symlink/special or inaccessible files preserve the previous compiled styles with a bounded warning. Save uses the captured64hex SHA-256 revision, a synced same-directory temporary file, a second fingerprint check and first-create no-clobber. Existing file contents are preserved on stale/invalid refusal. As in other preference stores, the final fingerprint/rename gap is not an adversarial filesystem compare-and-swap, and OS filesystem sync cannot be forcibly cancelled.

Worker API: `Worker::start(path_for(database))`, `.handle()`, `.join()`; Handle offers `.snapshot() -> Arc<Snapshot>`, `.request(Command::Reload | Save { expected_revision, settings })`, `.shutdown()`. Each paint frame cheaply captures one immutable Arc; repeated snapshot calls share it until a new file/settings/warning generation is published. Snapshot has ready/settings/revision/profiles/warning and `.style(view,column,value,row,selected) -> Option<Applied>`. Applied contains RGBA colors, centering booleans and optional margins. App/backend lifecycle and each table's explicit column map/application are a separate integration step. Existing semantic colors remain until that step; this module alone does not establish complete UI highlighting parity.

Four focused core cases cover ordered exact/substring styles, alignment replacement and selected-state protection; source column mappings, named/ARGB/high-depth colors and limits; stale/corrupt/symlink preservation; and worker last-good Reload plus normal fixture shutdown. No real daemon, network, GPU, package or compositor proof is claimed by these cases. Retained logs live under `.verification/highlighting-20261016/`.

Final focused four-case log: `.verification/highlighting-20261016/final-core.log` (existing compiled test binary, 0.05 s runtime); strict no-default-features all-target Clippy passed, including raw destination hardening (`core-clippy-final.log`). Exact owned source/Cargo hashes are in `result.json`.

The final Arc snapshot refinement passes all four cases again (`arc-core.log`, compile2.08 s/runtime0.05 s), including pointer sharing and retained prior generation. Earlier strict Clippy predates that narrow API refinement; the shared normalization check is tracked separately.

## Native rendering integration

`src/app/highlighting.rs` applies the compiled RGBA, background, centering and
bounded padding through GPUI's existing `Div` styling API. It creates no renderer,
font engine or I/O path. Every virtualized list captures an immutable `Arc` once
for its visible render range. History's Kit DataTable delegate captures the Arc
from its parent render and mirrors the existing `SelectRow` event; it does not
try to read a mutably rendered TableState. All views pass their actual selected
row to the matcher before applying any custom properties.

The explicit native column contract is:

| View | Keys |
| --- | --- |
| History table | Every `history::query::Field::key()` selected for display; all native History fields are available to row matching. |
| History cards | `time`, `action`, `process`, `destination` (formatted endpoint), `protocol`, `reason`; all native History fields also participate in row matching. |
| Rules | `name`, `action`, `duration`, `enabled`, `process`, `description`, `cache_status` |
| Nodes | `name`, `status`, `interception` |
| Applications, Hosts, Addresses, Ports, Users, Protocols | `key`, `hits`, `nodes` |
| Alerts | `severity`, `node`, `time`, `read`, `summary` |
| Firewall | `position`, `id`, `target`, `enabled`, `description` |
| LiveSockets | `title`, `endpoints`, `metadata` |

Matching uses the displayed text of each mapped cell. Human labels follow the
active language; rule actions, durations, addresses, paths, node names and daemon
payloads retain their original values. Source Qt column indices are never
assumed to match these keys. Import callers must supply the explicit mapping
already required by `import_source`. Alternate native cards and tables have
different visible columns, so their listed mappings are intentional.

The dashboard's **Reload table styles** button (`highlighting-reload`) submits
one request to the existing four-slot worker and prevents a second UI reload
while it is pending. It refreshes all native windows after completion. Startup
and Reload read the bounded native file; malformed or changed content retains
the last good compiled profile and displays the worker warning. There is no
polling watcher and no GUI rule editor; the pinned source's `configure()` is
itself unimplemented. Users can edit the documented data-only native file and
reload it. Row/window dimensions remain native and fixed where the existing
view fixes them; configured padding is bounded and clipped within cell layout.

The first combined all-features desktop check passes in
`.verification/highlighting-20261016/native-desktop-check2.log` (5.17 seconds).
This is build evidence only; subsequent native GPU/input, selection precedence,
invalid-file last-good and explicit Reload verification are tracked separately.

## Row preparation and render cost

`Snapshot.enabled(view)` lets default/disabled profiles bypass hidden-field text
construction. `prepare_row(view,row,selected)` returns an owned Copy PreparedRow;
`style_prepared(view,column,value,prepared)` performs only cell matching and then
merges the already-computed row overrides. Keep the same immutable Arc generation
for preparation and all its cells. Selected/oversized rows remain ineligible.
The compatibility `.style()` path is retained for single-cell callers.

A focused debug-binary measurement of12 rows ×24 displayed cells and32 unmatched
row rules took150,825µs when rescanning the row for every cell versus6,240µs
preparing once per row (observed24.17× for that fixture). These are pure matching
costs, not real frame latency, release performance or RSS. Four semantic/store
cases and the explicitly invoked cost measurement pass; the measurement is
ignored in ordinary tests and asserts no unstable timing threshold. Exact hashes
and bounds are in `.verification/highlighting-20261016/prepared-row-result.json`.
The native cache adoption and configured-style pixel proof follow separately.

The resource refinement prepares row substring matches once before styling its
cells. The History DataTable additionally keeps at most 64 visited rows of field
strings and owned prepared results. Weak Arc identity checks invalidate the
cache on page/metadata or style-generation replacement, even when the row id or
persisted revision is unchanged. The cache releases entries on an empty/new
page. Default/off profiles and selected History rows return before constructing
hidden-field strings. This cache does not retain the backing page or settings
through a strong Arc.

The focused desktop cache regression passes in `native-cache-test3.log`
(24.46-second test build, under 0.01-second runtime). It checks same-id metadata
replacement, two style generations with the same absent persisted revision,
selected-state protection, 64-entry bound and page-clear invalidation. Initial
failed test builds are retained separately: macro-import shadowing and the
verifier's now-repaired tuple-click/private-read harness errors. Core matching
and the pure prepared-row cost fixture are separate evidence in
`prepared-row-result.json`; its 24.17x fixture ratio is not an application frame
or RSS measurement. Native rendering/input proof remains pending until the
focused GPU journey completes.

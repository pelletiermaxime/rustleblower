# Counter selection and selected output

The pinned OpenSnitch generic table supports multiple rows, Shift ranges and
Select all, with selected-row copy separate from full-table file export
(`customwidgets/generictableview.py:283-307,343-411,457-620` and
`dialogs/events/views.py:357-410` at the pin in `upstream-reference.md`).
Statistics already uses daemon-reported counters, a global sort and a private
disk-backed snapshot. Retained connection events have different provenance.

The native selection extends that existing immutable counter snapshot. The
first checkbox mark or Select all freezes the displayed snapshot. Previous/Next
page keep it; Latest, category, node or sort/filter changes clear marks. Clear
selection keeps the frozen snapshot until Latest. A focused row continues to
provide matching-history and reviewed local-reset actions. Row checkboxes manage
the selected output; the reset action is labelled Reset focused value.

Selections retain the exact snapshot Arc, one optional inclusive position range,
an anchor and at most 1,024 individual inclusion/exclusion overrides. They do not
retain copied keys, read a file on the UI thread or expand all matching rows into
an ID collection. Positions remain stable because the snapshot never changes.
The existing snapshot has at most 10,000 keys and 4 MiB on disk; only 200 rows are
read for a page. Descriptor capacity refusal preserves prior marks.

The export window captures the snapshot and selection when opened. Selected
output can choose Value, Hits since local reset, Last reported total and Nodes,
in CSV or JSON Lines. These are data-column choices, separate from the row
checkbox. Reopen the export window to capture a different selection. Full-match
export and matching-page CSV copy keep their existing behavior.

Output reads/encodes captured pages on a background executor. A single shared
encoder implements existing full export and selected output. File output is
limited to 16 MiB and clipboard output to 4 MiB; unknown/duplicate/empty column
sets are refused. Destination checks reserve application files; replacement is
explicit. Errors, capacity failure and Stop preserve an existing file. The
export body scrolls while Stop/Close stay in the footer. Forced view destruction
requests cancellation; ordinary window close retains a busy export.

## Current evidence

Three focused model/persistence/export cases pass in
`.verification/statistics-selection-core-initial.log` (4.88-second build,
0.22-second test runtime). They cover 650-key cross-page ranges in all four
global sorts; individual exclusions; changed range anchors; chosen columns;
SQL counter deletion after capture with unchanged output; 1,024-mark refusal;
all/range constant descriptor size; 4 MiB clipboard refusal with successful file
output; cancellation, invalid columns, protected destinations and existing-file
preservation.

The four existing Statistics protocol/storage cases also pass against owned
disposable sockets in a private loopback-only namespace: 3.72-second build and
0.17-second runtime in `.verification/statistics-selection-regressions.log`, with
namespace evidence in `.verification/statistics-selection-regressions-network.json`.
These cover absolute counters, baseline/reset/restart persistence, frozen page
and output behavior, exact reset scope and permission independence under storage
contention.

The linked controls also pass the concrete all-features/all-targets native check
in `.verification/i18n-phase3/native-check.log` (8.55 seconds). Fourteen actual
GPU/input frames pass in `.verification/headless/20261017T043959Z-14963128`
(38.65 seconds). They include a 201-row Shift range across pages, 349 selected
rows after Select all and exclusion, two chosen output fields with identical
file/clipboard contents, frozen output while new counters arrive, and an
independent permission reply. Aggregate hashes, immutable binary/source scope
and exact bounds are in `.verification/statistics-selection-gpu-result.json`.
No packaged or compositor proof is implied. Table/header presentation persistence, grouped
retained-history details and UID/service lookup are separate remaining source
surfaces. This selection/output slice keeps the existing reviewed reset API; it
does not establish a source contract for a new multi-value reset action.

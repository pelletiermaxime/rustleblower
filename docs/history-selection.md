# Captured history selection

History supports individual marks, a Shift-click range and Select all matching.
These are separate from the focused event used by Details. Both the card list
and the pinned Kit table use the same selection. A checkbox inside the first
table cell preserves the existing data-column/sort/export mapping.

The first mark captures the current filters, ordering and arrival ceiling.
Older and First page browse that captured view without clearing marks. Clear
selection keeps the view frozen; Latest clears selection and resumes following
new reports. Changing a filter or sort also starts a new view and clears marks.
Refresh respects a captured ceiling. Selection is session state, not a saved
history preference.

Shift-click extends from the most recent ordinary mark and replaces the active
range, retaining separate manual marks outside it. Select all matching stores a
query descriptor rather than an expanded list of IDs; individual unchecked rows
become exclusions. The descriptor is bounded to 1,024 individual overrides and
128 KiB, with an 8 KiB limit per sort key. Exceeding a limit preserves the prior
selection. A range uses inclusive sort-value/ID endpoints, so ties, missing
metadata and ascending/descending sorts have the same membership as SQL paging.

Open Filters, columns & export after selecting events, then choose Export.
Export selected events and Copy selected events use that captured selection,
the chosen data columns and CSV or JSON Lines format. Reopen the tools to
capture a changed selection. Existing full-match export and matching-page copy
remain available. Selection checkboxes do not become data columns.

Selected output creates a private SQLite backup on a background executor, then
streams matching retained rows from it. It never loads an unbounded list of
selected IDs or holds the live database open while encoding/writing output.
Clipboard output is limited to 4 MiB; file output to 256 MiB. The existing source
backup limit is 4 GiB with a 15-second acquisition deadline. Stop or closing the
tools requests cancellation; errors and cancellation preserve an existing target.

The captured ceiling excludes later arrivals; it does not prevent retention from
removing old events. An explicitly marked positive ID that has expired causes
output to fail before publication. Range/all selection includes rows still
retained in the captured query at backup time. This history remains a bounded
operational view, not a lossless audit log.

## Verification boundary

Four focused integration cases in `tests/history_selection.rs` pass together
with the three existing history-export cases. Evidence is retained in
`.verification/history-selection-core.log`. They exercise all twenty sortable
fields in both directions with ties and missing metadata, a range crossing
200-row pages, individual inclusions/exclusions, range contraction, frozen
arrival membership, chosen-column CSV/JSON Lines, descriptor count/byte limits,
clipboard overflow, expired explicit marks, cancellation and preservation of an
existing output file. These are core query/persistence/export checks.

The actual nine-frame GPU/input journey passes in
`.verification/headless/20261017T042722Z-ed034259/result.json` (37.13 seconds),
using the immutable executable/source snapshot recorded in
`.verification/history-selection-immutable.json`. Actual pointer and Shift input
select a range across a 200-row boundary; First page retains marks. Selected
JSON Lines contains exactly 202 range/manual marks; Select all minus one produces
204 CSV rows with exactly two chosen fields and identical clipboard output. A
newly arrived event is excluded by the captured ceiling. Details, a real AskRule
Deny reply, the pinned table checkbox and Latest/filter/sort clearing all pass,
with owned process/socket cleanup. The first harness compile needed a String
conversion fix; no production change was needed after this native checkpoint.
Native compositor and packaged selection input remain separate checks.

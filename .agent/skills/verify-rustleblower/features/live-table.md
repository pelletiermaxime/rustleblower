# Live socket table, selection and saved view

Read [source and bounded contract](../../../../docs/live-table-plan.md). The Kit
table exposes thirteen columns, with typed global sorting before 200-row paging.
Marks, ranges and all-matching selection capture one session/revision. Frozen CSV
with chosen columns or raw socket JSON keeps that scope while updates continue.
Incoming KitDefault maps to Descending for a reversible two-state sort cycle.

`live-view-tools` works before monitoring. Saved numeric filters, interval,
search, sort and 1–13 visible columns/widths use `<database>.live-view.json`.
Loading and saving run off the UI thread and reject stale revisions. Loading a
saved view leaves monitoring stopped. Marks use `("live-mark", raw_index)`;
column indices 0–12 are Command, Process, State, SourcePort, SourceIP,
DestinationIP, DestinationPort, Protocol, UID, PID, Family, Interface and Metadata.
Tools show the captured row count/session/revision, typed format, bounded path
and cancellation.

Run `python scripts/verify-headless.py --feature live-table`. Require numeric
header sorting in both directions, Shift selection across pages, exact frozen
CSV/JSON output, save/reopen/stale/reload behavior, stopped restoration and
independent permission input while a live update is held.

The journey also double-clicks the first column's resize handle and requires a
changed rendered width. Kit 0.8.0 measures at most the loaded 200-row page, with
bounded text shaping. The fitted width enters the existing staged presentation;
use the view tools' Save to persist it. This is Kit input/layout proof; compositor
pointer delivery and physical font metrics require a native run.

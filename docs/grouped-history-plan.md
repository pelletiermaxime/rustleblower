# Grouped retained-history detail scope

This is the next source-backed detail slice, not a claim of implemented grouped
views. The reference is OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`:
`ui/opensnitch/dialogs/events/config.py:84-205` defines detail columns and
`queries.py:511-594` defines category grouping.

The top-level native Statistics categories already show daemon-reported
counters with global sorting and an exact matching-history action. The source
detail views additionally aggregate retained connection rows, displaying a hit
count and latest time for each combination below. These counts need an explicit
retained-history label because telemetry retention/deduplication means they do
not equal daemon counters.

| Detail entry | Bound scope | Source grouping keys |
| --- | --- | --- |
| Node | Exact recorded node | executable, command, UID, source IP, destination IP/host/port, protocol |
| Rule | Exact node and rule name | executable, command, UID, destination IP/host/port |
| Host | Exact hostname | PID, executable, command, source IP, destination IP/port, protocol, action, node |
| Application | Exact executable | source IP, destination IP/host/port, UID, action, node, PID, command |
| Address | Exact destination IP | PID, executable, command, source IP, destination port/host, protocol, action, UID, node |
| Port | Exact destination port | PID, executable, command, destination host, source/destination IP, protocol, action, UID, node |
| User | Exact UID | PID, executable, command, source IP, destination IP/host/port, protocol, action, node |

Implement with the existing typed history predicates and bound values, never
interpolating a recorded node, rule, hostname or path into SQL. Missing metadata
must remain missing rather than becoming UID/PID zero or an empty known value.
When fields outside the grouping are displayed, explicitly use a deterministic
latest representative (time, then event ID), preserving access to that event's
existing Details/current-rule/process actions. Avoid arbitrary values from a
bare SQLite GROUP BY.

A small implementation sequence:

1. Add a private disk-backed grouped snapshot using the existing bounded backup
   path, fixed typed grouping definitions, 200-row reads and a single background
   operation permit. Capture a ceiling/query once; keep output on disk and stream
   selected/full exports. State source/snapshot/output/time budgets explicitly.
2. Add a singleton Kit table detail window with hit/latest-time columns,
   meaningful global sort, paging, selection and export. Reuse existing event
   Details by representative ID. Expose it from the focused Statistics category
   value and from node/rule actions, carrying exact captured node scope.
3. Prove multi-node ties, counts, missing metadata, bound literal input,
   deterministic representatives, frozen page/export contents, retention and
   independent permissions with owned fixtures; add actual GPU input for the
   category/node/rule entry points and Back/Close behavior.

Keep the raw matching-history action available. No grouped counter view owns
daemon counters, changes interception or edits firewall policy.

## Current implementation boundary

The private grouped snapshot and selected output core are implemented in
`src/history/grouped.rs` and `src/history/grouped/output.rs`. Two focused
model/storage/output cases pass in `.verification/grouped-history-output-core.log`
(1.53-second runtime). They cover all eight typed scopes, 3,900 owned events on
two nodes, 200-row paging, global numeric sorting, deterministic representatives,
literal SQL-looking values, frozen output, cancellation and selected file/clipboard
equality. A separate page-size regression proves the existing 8 GiB backup cap
using the actual SQLite page size, including a 64 KiB database.

The core keeps one retained private snapshot, bounded descriptors and output,
and background SQL cancellation/deadline checks. Protocol is the eighth detail
category, alongside the seven source scopes listed above. No grouped detail
window or category/node/rule entry point is linked yet. That UI work is deferred
while the existing application receives visual cleanup and final stabilization;
these core tests do not establish native input or grouped feature completion.

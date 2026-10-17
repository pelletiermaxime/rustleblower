# Typed rule catalogue search

Reference: OpenSnitch commit `a1353848ba1b660320e90cefea782c3fba272c00`,
`ui/opensnitch/dialogs/events/queries.py::rules_opts/rules_opt_map` and its advanced
operator translation. The source exposes created/time, name/description/node,
enabled/action/nolog/priority/duration and root operator type/operand/data.
Rustleblower implements the useful typed field predicates through a shared
bounded native filter window and predicate. Arbitrary SQL/expression text is
never accepted.

## Native controls

Application rules → Conditions opens a reusable native window. Choose a field,
comparison and value, then Add condition. Conditions remain staged until Apply;
Close discards unapplied edits when the window is closed. Apply clears stale row
selection/paging and combines conditions with the current quick search and
All/Permanent/Temporary scope. Node/session changes reject an old window's Apply
instead of applying its captured scope to another node. These filters stay in
the current application session; other-page presentation persistence remains a
separate gap.

Fields include Last received by UI (UTC), Created, Rule name, Description, Node, Enabled, Action, Logging
disabled, Priority, Duration, Root operator type, Root operand, Root operand data
Any operator operand, Any operator data and Executable. Boolean values use 0/1. Created compares the actual protocol
`Rule.created` Unix seconds; RFC3339 dates must have whole-second precision.
The pinned daemon's `daemon/rule/rule.go::Serialize` constructs the RPC field with
`created.Unix()` (line 148), after parsing the rule file's separate RFC3339
Created string. File JSON dates and RPC integer units are distinct formats.
Zero remains the raw protocol value, because the protocol cannot distinguish an
omitted timestamp from an explicit zero. No arrival/first-seen time is invented
for existing cached rules.

The local receipt field implements the source Time intent without reusing
`Rule.created`. Schema10 adds a separate `rule_observations` table keyed by node
and rule name. Inventory arrivals, confirmed catalogue changes and newly retained
matching daemon events update integer UTC seconds. Duplicate event replays do not
advance it. Old schema9 rows remain unknown until a fresh observation; synthetic
history fixtures and unanswered prompts do not invent daemon observations. The
metadata table can contain only cached rules and deletion triggers remove orphan
keys; a page retains at most200 timestamps. Timestamp changes never rewrite the
protobuf or submit policy changes. Not reported/Reported distinguish untouched
legacy rows; whole-second RFC3339 and Unix seconds use the same numeric operators
as Created. Full and selected exports use this predicate; extra UI metadata is
not added to exported daemon rule documents.

Root operator fields still inspect only the outer operator. The additional Any
operator operand and Any operator data selectors inspect nonempty strings across
the existing bounded tree, including nested leaves and unknown future kinds.
Positive conditions match any value; Does not contain/Does not equal require no
matching value anywhere, so an unrelated sibling cannot make an excluded rule
match. Not reported means no nonempty value anywhere in the tree. This provides
the useful nested-data search represented by the source's JSON-list text without
changing root-field semantics, flattening the daemon document or evaluating
regexes. Arbitrary user-programmed leaf expressions remain unsupported.

Text comparisons are literal Contains/Does not contain/Starts with/Ends with,
ASCII-case-insensitive Equals/Does not equal, and case-sensitive Exact match.
`%`, `_`, quotes, backslashes and SQL-looking strings remain ordinary text.
Before/After and At most/At least compare numeric fields numerically and other
fields lexically, with inclusive boundaries for At most/At least. Missing root
operators or empty text fields are unreported; use Not reported/Reported to find
them. Numeric/boolean protocol fields remain reported, including zero/false.

## Finite groups and shared matching

Match groups chooses All (AND) or Any (OR); each group's own All/Any control
combines its conditions. Start a new group with the next condition begins a new
group. The representation is the same finite two-level Logic used by history:
at most eight total conditions, 256 bytes per value and eight nonempty groups
that partition conditions in displayed order. The default remains an empty
filter list with existing quick-search/scope behavior. The newer optional
condition-number formula supports nested AND/OR directly under a 256-byte,
eight-condition/eight-parenthesis-level/fifteen-node cap. Every condition is
referenced once; general SQL/functions remain unsupported. Read [formula
contract and separate evidence](nested-search-formulas.md), since the original
eight-frame proof below predates that addition.

`rules::query::Predicate` prepares validation/comparison values once. The exact
same predicate handles quick search, permanent/temporary scope and typed groups
for catalogue pages and multi-node folder capture. Selected/full-filter JSON,
file/folder/clipboard exports and policy reviews already traverse the catalogue
query through `transfer::visit_snapshot`; they therefore inherit the same
matching expression. Explicit selected-name mismatches still fail. Multi-node
capture evaluates typed Node against each actual selected node, rather than
the currently displayed source-node name.

The catalogue SQL only binds fixed node/name keyset parameters and orders by
name. Matching streams decoded rules until it has 201 matches, returning 200 plus
the existing next-page indication. It does not limit candidates to 201 before
filtering. A scan admits at most 10,000 rules, 2 MiB total protobuf bytes and
64 KiB per rule and an 8 MiB matching-page allocation budget; decoded validity
and cached-name identity checks
remain enforced. Only one candidate and the bounded matching page are retained.
At most eight boolean results/groups are evaluated per candidate. There is no
new metadata migration, persistent schema, file watcher or UI-thread I/O.

Policy observation, ACK/review and stale snapshot guards are unchanged. Awaiting
prompt proposals remain visible/exportable but cannot become confirmed editable
inventory merely because a predicate matched. Filtering reads cached protobuf
bytes without rewriting them; opaque root type/operand/data and existing policy
fields remain intact in exported JSON. Existing import validation still refuses
unsupported operator kinds. Unknown future protobuf tags stay unchanged in the
cache and initial raw-wire spooling; the established JSON interchange format does
not represent arbitrary unknown protobuf tags.

Visible catalogue pages retain their existing revision-reset semantics when the
daemon inventory changes. Transfer traversal uses one SQLite snapshot across all
pages; multi-node capture freezes its raw documents before user review and later
filesystem publication. Filtering does not weaken these existing boundaries.

## Verification

`tests/rule_search.rs` covers all typed field families, numeric/time/boolean
boundaries, literal metacharacters, expression-injection strings, group/value
bounds, more than 200 matching rows, selected/full-filter consistency, unknown
cached wire preservation, real-node multi-node matching, immutable captured
exports and pending-observation review refusal.

The predicate module's unit test modifies a later matching rule while a grouped
multi-page transfer traverses one SQLite snapshot. It requires the original
complete filtered snapshot and separately confirms the live predicate sees the
changed rule. Focused core/build results and new native GPU/input proof must be
reported separately; previous rule screenshots do not prove the new controls.

The focused run passed eighteen integration cases (four new search, five rule,
five rule-transfer and four multi-node export cases) plus the grouped snapshot
unit case. Existing socket fixtures ran in an owned user/network namespace.
Commands, logs, namespace proof and source hashes are saved under
`.verification/rule-search-implementation-20261017T023808Z/result.json`.
The first new fixture incorrectly expected import to accept unsupported future
operator kinds; the corrected tests inspect exact exported JSON and cached wire
bytes while requiring that established import refusal to remain intact. The new eight-frame background GPU/input journey passes in
`.verification/headless/20261017T024640Z-55e71c6e/result.json`. Actual Kit
controls reject an invalid boolean, construct two groups using action/enabled
and description/created filters, verify exact cached rule identities, change
root/inner All/Any, reopen the applied session, discard unsent staging, export
exact full protobuf values to JSON, remove a condition and clear. All screenshots
and owned process/socket cleanup pass. Aggregate and scoped hashes are retained
in `.verification/rules-advanced-result.json`; native compositor and kernel
enforcement remain separate.

## Receipt/nested field checkpoint

`.verification/rule-receipt-tree-result.json` records31 passing cases across six
affected integration suites. Three added cases prove650-rule receipt filtering
across pages, selected/full and two-node snapshots; actual matched-event clock
updates with uncached-name refusal; schema9 migration with unknown receipt and
unchanged unknown protobuf bytes; and deep leaf inclusion/exclusion without a
negative sibling leak. These core/SQLite tests run in an owned network namespace. The expanded14-frame GPU/input journey also passes, covering invalid receipt dates, positive receipt filtering, nested operand/data inclusion, exact nested JSON export and whole-tree negative exclusion. Exact images/source and the intentionally separate nested mutation-admission limit are in `.verification/rule-receipt-tree-gpu-result.json`.

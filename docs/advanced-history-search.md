# Typed advanced history search

Reference: OpenSnitch commit `a1353848ba1b660320e90cefea782c3fba272c00`,
`ui/opensnitch/dialogs/events/queries.py`, especially its operator constants,
connection field map and `advanced_search`. The source exposes working-directory
search, not-contains, starts/ends-with, inclusive comparisons and compound
AND/OR expressions. Rustleblower supplies these through native typed controls
and bound SQLite parameters. It does not accept arbitrary expression text or SQL.
The group controls are complemented by a bounded condition-number formula for
nested AND/OR expressions; see [formula contract and separate proof](nested-search-formulas.md).
Arbitrary SQL, functions and unbounded expressions are outside the native contract.

## User flow

History → Tools → Filters offers Working directory alongside the existing typed
fields. Text comparisons include Contains, Does not contain, Starts with, Ends
with, Equals, Does not equal and case-sensitive Exact match. Comparison text is
literal: `%`, `_`, quotes, backslashes and SQL-looking text do not become wildcard
or SQL operators. SQLite's existing ASCII case folding applies to comparisons
other than Exact match; full Unicode collation is not introduced by this slice.

At most / at or before and At least / at or after include the boundary value.
Numeric fields use validated integers; ports remain 0–65535 and boolean fields
remain 0/1. Time accepts integer Unix nanoseconds or RFC3339 with an offset.
Existing strict before/after and Not reported/Reported controls remain available.
Empty or unavailable working directories are unreported and do not match text
comparisons, including Does not contain. Use Not reported explicitly to find them.

Choose Match groups: All (AND) or Any (OR). Within each group, its own All/Any
button chooses how its conditions combine. Check Start a new group with the next
condition before adding the first condition of another group. For example:

```text
Match groups: Any (OR)
  Group 1: All (AND)
    UID equals 1001
    Working directory starts with /work/project
  Group 2: All (AND)
    Destination port at least 8000
    Destination host ends with .internal
```

This means `(UID = 1001 AND cwd starts with /work/project) OR
(port >= 8000 AND host ends with .internal)`. Main search and action restrictions
always apply alongside the complete grouped expression. Removing a condition
retains the other groups and removes a group when its last condition disappears.
Clear conditions restores the default All/AND expression.

## Bounds and compatibility

The original eight-condition and 256-byte-per-value limits remain. A finite
two-level representation supports up to eight groups; every nonempty group
consumes the next `count` conditions in displayed order. Counts must partition
all conditions exactly once. Conditions cannot be duplicated by reference and
the ordinary group controls remain a finite two-level model. The optional
formula adds a separate bounded parser: eight conditions, eight parenthesis
levels, fifteen AST nodes and 256 ASCII bytes. It handles nested AND/OR
expressions directly without expanding them into duplicated groups. SQL template
input and unbounded trees are refused. The native editor cannot publish empty
groups or a ninth condition. The original seven-frame evidence below predates
the formula; its focused input proof is recorded separately.

`Query.logic` contains a root `mode` (`all`/`any`) and groups containing `count`
and `mode`. Empty groups use the root mode for the existing flat filter vector;
the default remains All/AND. Existing query callers using `Default` retain their
behavior. History presentation adds an optional, defaulted `logic` field while
retaining the existing saved-settings version and migrations from versions 1–3.
Group state is saved/reopened with filters, sorts and column widths; row IDs,
keyset cursors and frozen ceilings remain transient.

SQL is built exclusively from enumerated field/operator/join templates. All
comparison values are bound parameters. Starts/ends and contains use literal
string operations, avoiding LIKE wildcard interpretation. The same expression
builder serves the global matching-set sort, frozen keyset paging, bounded page
clipboard and all-match file export. Export retains its existing private SQLite
snapshot semantics: it exports all matching rows at export capture time rather
than the current displayed page. Its copy completes metadata backfill before
evaluating the captured expression.

## Working-directory migration and responsiveness

New events store at most 4096 UTF-8-safe bytes of `process_cwd` in bounded JSON
metadata; truncation is flagged. This adds no SQLite column or user-version bump
and retains schema-9 compatibility. Existing bounded detailed snapshots supply
cwd for already-indexed rows as well as original NULL legacy metadata.

A per-row metadata `schema: 1` marker makes the upgrade resumable. The existing
SQLite worker upgrades at most 32 records per transaction between commands.
Previously indexed values and unknown JSON fields are retained; only cwd, the
schema marker and truncation flag are extended. Byte accounting changes by the
serialized metadata size delta. Existing completed chunks are skipped after
restart. Missing detailed snapshots retain unknown cwd rather than inventing it.
The migration record cap is 32 KiB for legacy metadata and the existing detail
payload constraint remains 24 KiB.

Indexing feedback remains visible. Applying advanced filters and metadata sorts
waits for indexing in the native UI. The actual SQLite query path also rejects
metadata filters/sorts while legacy metadata remains, so early or failed
migration cannot silently omit matching cwd rows. Basic summary history stays
available. This work runs on the existing writer; permissions retain their
independent admission/reply path and do not wait for indexing or export.
Each owned writer/export connection records completed indexing in a one-row
temporary SQLite marker. Subsequent metadata pages/refreshes check that marker
instead of repeatedly scanning every retained JSON row. It disappears with the
connection, is not copied into exports, and adds no persistent schema change.

## Verification

`tests/history_advanced.rs` covers literal metacharacters and expression-injection
strings, inclusive number/time boundaries, two-level AND/OR predicates across
multiple globally sorted pages, stable ceilings after ingestion, all-match JSON
export, old settings/reopen, invalid group partitions, source/value bounds,
already-indexed cwd backfill, interrupted-chunk markers, unknown legacy rows,
preserved indexed/extension fields and stable byte accounting after restart.

The metadata unit test proves the 32-row transaction cap, unfinished-query
readiness error, restart/resume, UTF-8-safe cwd truncation and byte accounting.
Existing history tools/presentation tests remain regression coverage. Core/build
success must be reported independently from the expanded native GPU/input
journey; older history screenshots cannot prove the new controls or predicates.

The focused run passed 11 integration cases plus the metadata unit case; the
existing RPC regression used an owned user/network namespace. Shared strict
desktop Clippy also passed. Exact commands, logs, namespace proof and source hashes
are retained in
`.verification/history-advanced-implementation-20261017T022241Z/result.json`.
The focused `history-advanced` background GPU journey passed seven checkpoints in
`.verification/headless/20261017T023819Z-1739fc2d/result.json`. Actual Kit input
constructed `(cwd starts /work/a AND port >= 443) OR (cwd not contains /work/a
AND host ends .test)`, checked the exact four matching rows, switched inner/root
All/Any modes, exported the same four records with cwd, independently reopened
the saved expression, removed a condition without corrupting group partitions,
and cleared the query. All screenshots exist and the owned process/socket were
cleaned up. The aggregate `.verification/history-advanced-result.json` records
source hashes and proof boundaries. Native compositor proof remains separate;
new nested formula code/core proof is described in [its own scope](nested-search-formulas.md).

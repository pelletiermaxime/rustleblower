# Nested history and rule expressions

The pinned OpenSnitch `ui/opensnitch/dialogs/events/queries.py:145-200` accepts
compound advanced expressions by translating known connection/rule properties.
Rustleblower's existing two-level group controls could not represent every
parenthesized AND/OR expression. The shared native `Logic` model now has an
optional condition-number formula without accepting SQL text.

For example, four displayed conditions can use `4 OR (1 AND (3 OR 2))`.
AND binds before OR; parentheses override that precedence. Upper/lowercase
keywords and `&`/`|` are accepted. Every displayed condition must appear exactly
once. Duplicate references, missing/out-of-range numbers, trailing syntax and
SQL-like input are refused. To reuse a predicate, add another explicit condition
within the same eight-condition limit.

The formula is at most 256 ASCII bytes, with eight conditions, eight nested
parentheses and fifteen AST nodes. Filter values retain their existing 256-byte
bound and typed validation. Negation remains explicit in condition operators
such as Does not equal/Does not contain; this grammar has no unary NOT or general
SQL expressions/functions. These limits are native resource boundaries, not a
claim to execute arbitrary source SQL.

The history SQL builder visits leaves in formula order while appending bound
values in that same order. Merely rearranging SQL fragments after binding would
silently assign another condition's values to a placeholder. A pure parser case
and actual indexed-history integration case assert reordered leaves. No typed
condition data is interpolated into SQL. The rule catalogue prepares the same
bounded AST once and evaluates it against each candidate's typed predicates.
Paging, selected/full-filter operations and exports use that common predicate.

The history and rule filter windows offer a bounded Kit Input below the numbered
condition list. A nonempty formula overrides and disables the group mode
buttons. Empty restores their existing behavior. Adding/removing conditions
clears the formula with feedback because condition numbers changed. Invalid
Apply preserves the main view's previous expression. History's existing saved
presentation serializes the optional formula; absent fields preserve old group
and all/any behavior. Rule filters remain session state as before.

Executed verification:

- Two parser cases pass in `.verification/search-formula-parser.log`: nested
  truth tables, precedence, reordered SQL leaf bindings, maximum depth/node
  boundaries, duplicate/missing references, injection-like syntax and invalid
  counts/encoding.
- All ten cases in the affected history/rule integration binaries pass in
  `.verification/search-formula-core.log`. Two added cases prove exact matching
  across multiple pages, complete JSON Lines/rule export, selected-rule
  admission, saved history/reopen and invalid expression refusal without damaging
  subsequent queries.
- Desktop compilation passes in `.verification/search-formula-desktop-check.log`
  and strict core/all-target Clippy passes in
  `.verification/search-formula-clippy.log`.

The expanded history journey passes nine GPU frames (47.32 seconds) and the
rule journey passes ten (51.60 seconds), recorded in
`.verification/search-formula-result.json`. Actual input proves invalid Apply
preservation, reordered nested exact row sets, full export, saved/reopened
formula text, inert disabled group buttons and restored group behavior after
condition removal. All 19 PNGs and complete owned process/socket cleanup are
retained with a full 125-file precompile source snapshot. The first attempts
reached correct nested rows but used unsupported Kit disabled-state metadata;
the verifier replaced that assertion with actual attempted input and unchanged
results rather than changing production behavior. Screenshots were inspected
for readability and overlap. The earlier 238-case whole-core checkpoint
predates this addition. Final shared regressions, resource/package proof and
whole-app UI consistency remain separate.

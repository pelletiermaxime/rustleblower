# Required final UI and usability pass

This is a required final phase of the full Rustleblower goal, after feature
implementation (including suitable additions chosen from upstream issues/PRs and
official Little Snitch features). It does not interrupt an active functional
slice. The goal is incomplete until this pass has rendered/input evidence across
the app. Existing protocol/persistence correctness, bounded resources, prompt
independence and ownership boundaries remain required.

User feedback: there are too many nonsortable tables, insufficient meaningful
color and inconsistent pages. Assess the whole product rather than polishing one
screen and treating it as global proof.

## Shared native patterns

Inspect the pinned GPUI Kit components first; retain one backend. Establish a
small shared set of page layouts and reusable patterns for typography, spacing,
search/filter placement, primary/secondary/destructive actions, details/editing,
paging, loading/empty/error states and status feedback. Keep feature state and
I/O boundaries intact. Prefer direct inspection and predictable edit/save/cancel/
back flows; retain useful selection and search context when returning.

Color must communicate state: Allow/Drop/Reject, enabled/disabled rules,
connected/disconnected nodes, pending changes, warnings and errors. Use restrained
semantic tokens consistently across themes, paired with readable text/icons and
sufficient contrast. A colored badge alone is not an accessible state indicator.
Standardize row selection, hover/focus, row actions and keyboard navigation;
provide sensible column resizing where it helps.

## Sorting contract

Every meaningful table column needs a visible sortable header and sort direction.
Sorting must cover the entire matching dataset, not only the current page.
SQL-backed pages use bounded ordered queries and stable tie-breakers; frozen
snapshot and refresh semantics must stay clear. A bounded in-memory catalogue
may sort the complete admitted catalogue. Keep query strings, page sizes, queues
and retained rows bounded. Validate both directions, tied values, multiple pages,
filters and selection/context retention with actual header/keyboard input.

## Screen matrix and proof

| Surface | Required assessment | Status |
| --- | --- | --- |
| History and connection details | Global matching-set sorting, filter/search/table consistency, selection, details/back, paging/refresh/export | Final whole-app pass pending |
| Application rules, editor, import/export and bulk review | Meaningful sorting, semantic action/enabled colors, row/bulk actions, predictable editing and partial errors | Final whole-app pass pending |
| Statistics and drilldowns | Consistent category navigation, global sorts, chart/table hierarchy, reset/export feedback | Final whole-app pass pending |
| Nodes and daemon preferences | Connection/interception status, selected-node context, reviewed state changes, predictable save/back/errors | Final whole-app pass pending |
| Alerts and desktop notices | Severity/read state, sortable fields, detail/back, filtering and reviewed deletion | Final whole-app pass pending |
| Live sockets/process/resources | Meaningful bounded snapshot sorts, start/stop/pending states, inspection and empty/error behavior | Final whole-app pass pending |
| List subscriptions | Obvious navigation entry, immediately usable empty screen, normal UI Add/config creation, status/error/refresh/rule flows | Functional catalogue, downloads, single/group rules and membership review exist; final whole-app pass pending |
| System firewall | Cohesive chain/rule/policy/file pages, sortable meaningful lists, staged operation feedback, consistent form/review/cancel/back | Functional pieces exist; final whole-app pass pending |
| Permission popup and advanced review | Clear scope/action hierarchy and semantic colors, keyboard/input/font behavior, finite timers, independent responsiveness | Existing focused proof retained; final whole-app pass pending |
| General/history/popup preferences and tray lifecycle | Consistent setting groups, save/cancel/validation, opening/restoration and status | Final whole-app pass pending |

Verify with background GPU rendering and actual input first, followed by owned
native compositor input where relevant. Preserve screenshots/action evidence and
assess representative screens together for visual consistency in addition to
control-level checks. Build success or old screenshots cannot complete this pass.
Use isolated sockets/databases/HTTP fixtures; never alter the running daemon or
host firewall. Preserve ignored evidence and concurrent work. No publication is
authorized by this phase.

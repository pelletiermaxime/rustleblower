# Shared shell and History/Rules cleanup

This pass reduces permanent controls above the data. The dashboard has one
compact connection header, the existing seven navigation tabs, and a native
GPUI Kit Settings popover. The eight existing global actions keep their IDs
and handlers; socket/storage/fallback information is available inside the
menu, and prompt counters occupy a thin footer. Warnings remain visible.

History and Rules use quieter secondary buttons and clearer primary and
destructive actions. Search controls can wrap. Semantic action/enabled colors
use the active theme, retain text labels, and yield to configured highlighting
and native selection. Query, paging, selection and policy behavior are unchanged.

Static checks preserve feature control IDs, listener bodies and fixed row
heights against the retained pre-pass source. Actual rendered and input proof
is recorded separately under `.verification/visual-pass/`; a source edit or
build alone is not visual proof. Other feature toolbars/forms, success-message
treatment and a complete light/dark/minimum-size review remain outside this
small batch.

The first rendered/input checkpoint passes thirteen frames in
`.verification/headless/20261017T050057Z-351e46fd` (16.59-second test runtime).
It includes all eight settings action bounds at 780 × 560, actual menu and
Popup preferences input, and the existing window-state/permission/persistence
journey. The source manifest and immutable binary identity are retained in
`.verification/compact-shell-source/manifest.json` and
`.verification/compact-shell-immutable.json`. Screenshots were inspected for
menu fit and the normal Rules layout. This is background GPU/input proof;
compositor focus and packaged startup remain separate.

Strict Clippy for all targets with the headless feature passes in
`.verification/visual-pass/native-clippy-final.log` (11.07 seconds). The first
check found two redundant `AnyWindowHandle` conversions in the new verifier;
removing those does not alter product behavior. Subsequent GPU journeys retain
their own exact source/binary manifests.

The bounded regression batch also passes the existing general journey (48
frames), History selection journey (9 frames) and advanced Rules journey
(14 frames). Together with the window/menu journey, 84 screenshots are retained
in `.verification/compact-shell-result.json`; this count is evidence across
four flows, not a count of independent features or complete app coverage.
All tested product-source hashes match across the two retained binaries. The
general retry changes only stale verifier expectations for the human-readable
User ID/Process ID clipboard headings, preserving the exact copied values.

The Settings popover described above is superseded by the shared settings
window with a left category menu. Its layout and current verification contract
are documented in `.agent/skills/verify-rustleblower/features/settings.md`;
the earlier popover evidence remains historical.

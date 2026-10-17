# Shared settings window

Dashboard → **Settings** opens one 1180 × 820 native window, with a supported
minimum of 900 × 620. The GPUI Kit Sidebar groups Appearance/language, Startup
and Updates under General; Popup preferences and Reputation under Permissions;
and History, Rule catalogue, Raw downloads and Table highlighting under Data.
Only the active feature view is painted. At most nine feature entities are
created lazily and retained until the window closes, preserving staged inputs,
validation feedback and reviews across category changes.

Every page uses `settings::footer`: a fixed footer with the same spacing,
secondary Close button, primary Save button on editable pages, and bottom-right geometry. Context
operations (reload/check/cancel/import/export) live in the page content. Save
commits the active page. History retention, catalogue policy and download profiles
retain their two-step review, with the final Save in the same footer. The file-based
Table highlighting page has no editable preferences, so its footer contains only Close.

Appearance choices are staged, including color, density, palette and language.
One background Save publishes the complete document before applying the new
appearance. Failure preserves the applied theme and staged choices. Close discards
unsaved appearance choices; changing categories retains them. Close, the window
decoration and Ctrl+W close the window. Ctrl+Tab and Ctrl+Shift+Tab cycle categories.
The dashboard Settings
button, tray Startup, update/download notices, History preferences and Rule
catalogue preferences reuse this window and target the requested category.
Node-specific daemon configuration and captured reputation inspections retain
their separate contextual editors.

Run `python scripts/verify-headless.py --feature settings`. Require a passing
`result.json`, all listed JSON checkpoints, owned process exit and
scoped socket cleanup. The journey clicks every category without increasing the
window count, retains and saves a popup draft with independent file reopening,
retains a history draft and opens review without applying it, answers a real
AskRule while that review remains open, closes/reopens Settings and checks the
minimum size. It uses owned socket/database/XDG fixtures; this does not prove
native compositor behavior, system-daemon enforcement or packaged startup.

The existing popup-preferences, appearance, startup, history-preferences,
updates, reputation, raw-downloads and catalogue-policy journeys retain their
feature-specific contracts. Category navigation by itself does not establish
all of those features' save/network/protocol behavior.

The existing native History-preferences adapter targets the shared Settings
window title. Its compositor run must be repeated before claiming native proof
for this layout; background input proof alone does not refresh that claim.

The consistency journey additionally compares actual native button labels, sizes,
order and viewport-edge geometry on every page and the retention review. It
checks that Appearance has no disk/theme effect before Save, and that Close
discards a staged color while retaining the last successful save.

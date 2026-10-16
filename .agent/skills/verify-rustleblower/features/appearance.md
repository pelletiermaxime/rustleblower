# Native color appearance

Reference: OpenSnitch a1353848 (source 1.9.0),
`dialogs/preferences/sections/ui.py::load_themes/change_theme` and
`utils/themes/themes.py`. Upstream offers System plus installed Qt Material
palettes and a separate Qt-specific density scale. This slice implements native
System/Light/Dark color mode. Custom native theme files, density, language and
the final whole-app design pass remain in the full goal.

Architecture reference: FerryDock `src/app/appearance.rs` uses Kit Settings,
SettingPage/Group/Item and Theme::change. Rustleblower uses the same native
component pattern, with disk work moved to the background executor. The existing
FerryDock MIT notice remains under LICENSES/.

## User flow

Dashboard → **Appearance…** opens one reusable native settings window. Choose
System, Light or Dark. The choice applies immediately to all current/future
windows and is saved asynchronously. Current screen/filter/editor state remains
owned by the original feature views. There is no renderer replacement.

System uses the current native window appearance. An owned Base RootPlugin
observes native appearance changes for each window; its subscription drops with
the root. Color changes use Kit Theme::change, which reloads the registered
palette, synchronizes Base theme tokens and refreshes windows. Custom hardcoded
feature colors still belong to the required final contrast/consistency audit.

**Use System appearance** restores/saves the default. **Reload saved appearance**
replaces any session-only choice with the persisted mode. Closing keeps the
selected mode; it is not a staged dialog. A save error retains the chosen mode
for this session and explains that persistence failed. Another click explicitly
retries; there is no automatic retry loop.

## Persistence and bounds

The file is `<database>.appearance.json`, separate from popup/history settings.
It contains version 1 and `color_mode` (system/light/dark). Initial UI starts
with system colors and loads on a background executor. Permissions do not wait
for this read. One application global retains mode, native mode, one snapshot
and a bounded message; at most one load/save runs at a time. Controls disable
while that job is pending. There are no per-event file reads or growing queues.

Reads accept at most 4 KiB. Missing files use System without writing. Small
malformed, unknown-mode or unsupported-version documents remain unchanged and
fall back to System with visible feedback in settings. An explicit mode choice
can replace such a document. Symlinks, oversized/nonregular/unreadable files are
preserved and report errors. This is not a migration of Qt settings/XML palettes.

Raw content hashes reject files changed since the last load; new files use
no-clobber publication and existing files use a temporary-file rename. The
checks are not a generic atomic compare-and-swap against arbitrary external
writers. History's export path guard now reserves this appearance filename as
well, even when replacement is requested.

## Proof

`tests/appearance.rs` has three focused cases:

- Defaults and all mode roundtrips/reopen; sibling popup preferences unchanged.
- Invalid JSON, future version, unknown mode and unexpected fields use System
  without rewriting; explicit choice repairs the small file.
- Stale snapshots, oversized/symlink/invalid-directory cases preserve files.

The existing history export integration case now creates actual saved Dark
preferences, refuses this target even with replacement enabled and requires its
bytes unchanged. Use the owned-network wrapper for the full core suite as the
main skill documents.

`python scripts/verify-headless.py --feature appearance` retains eleven real
GPU frames using actual Kit Settings color/reset/reload buttons:

1. System settings on the test platform's native Light appearance.
2. Dark dashboard after an actual color choice.
3. Dark existing-rule editor with typed unsaved name/description.
4. Light editor with exact fields copied through Ctrl+A/C and retained; Cancel
   leaves the confirmed original and no saved unsaved-name rule.
5. Light permission from an actual synthetic AskRule RPC.
6. The same permission after Dark change, then actual Deny once reply.
7. An external stale-file edit refuses save while session colors still change.
8. Reload of that invalid document uses System and keeps the file unchanged.
9. Explicit choice repairs/persists Dark.
10. A fresh GPUI context loads Dark from the production store.
11. Actual reset returns to saved System/Light.

The general 48-frame and tray four-frame regressions cover shared navigation and
existing rule/permission behavior. Inspect the PNGs and require passed result,
real_gpu, scoped socket cleanup and owned process exit. The exact aggregate is
`.verification/appearance-result.json`. Rebuild desktop after core tests.

The test platform's appearance is fixed Light. This proves System initial/reset
resolution; it does not prove an actual native OS appearance-change callback.
Native compositor/input, packaged startup, all-page contrast, custom themes,
density and broad resource measurement remain separate. Tests never change the
user's real desktop theme or connect to the installed daemon/system socket.

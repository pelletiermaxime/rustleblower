# Native color appearance, display density and palettes

Reference: OpenSnitch a1353848 (source 1.9.0),
`dialogs/preferences/sections/ui.py::load_themes/change_theme` and
`utils/themes/themes.py`. Upstream offers System plus installed Qt Material
palettes and a separate Qt-specific density scale. Rustleblower implements native
System/Light/Dark color mode and the source's −20 through 20 density choices,
using a native 2%-per-step rem mapping (60–140%, default 100%). Installed data-only
native palettes are discovered from `<database>.themes/` and selected through
actual Kit buttons. Language, complete Kit theme-token customization and the
final whole-app design pass remain in the full goal.
See [density source and limits](../../../../docs/density-plan.md) for the native
mapping and fixed-pixel limitations; this does not claim exact Qt Material sizing.

Architecture reference: FerryDock `src/app/appearance.rs` uses Kit Settings,
SettingPage/Group/Item and Theme::change. Rustleblower uses the same native
component pattern, with disk work moved to the background executor. The existing
FerryDock MIT notice remains under LICENSES/.

## User flow

Dashboard → **Settings → Appearance** opens the Appearance page in the shared
settings window. Choose
System, Light or Dark, then use the shared bottom-right Save button. Choices
are staged until Save atomically publishes color, density, palette and language
on the background executor, then applies the saved appearance to all windows. Current screen/filter/editor state remains
owned by the original feature views. There is no renderer replacement.

System uses the current native window appearance. An owned Base RootPlugin
observes native appearance changes for each window; its subscription drops with
the root. Color changes use Kit Theme::change, which reloads the registered
palette, synchronizes Base theme tokens and refreshes windows. Custom hardcoded
feature colors still belong to the required final contrast/consistency audit.

**Use System appearance** stages the default. Close discards unsaved choices;
changing categories retains them. A failed Save preserves the applied theme,
keeps the staged choices, and reports the error. Reload in the page content
reads the current file/catalogues and discards the draft. There is no automatic
retry loop.

**Display density** has smaller/larger buttons and a scale/percentage readout.
After Save, the selected scale changes the native rem basis and actual Kit button
dimensions, rem text and spacing in current and future windows. The scale applies
to System as well as Light/Dark. **Default density** restores only density zero;
**Use System appearance** restores only color. Reload restores both saved choices.
Fixed-pixel geometry, table row heights and cell padding remain unchanged.

**Installed native palettes** exposes the discovered native JSON choices. Choose
one to stage its palette ID, declared Light/Dark base mode and current
density, then Save to apply it. Built-in color choices clear the custom palette; density choices retain
it. Reload rescans the catalogue and restores the saved selection. Missing saved
IDs retain their preference/density and use the saved built-in mode with visible
feedback. Catalogue contents remain a bounded snapshot until Reload.

## Persistence and bounds

The file is `<database>.appearance.json`, separate from popup/history settings.
It contains version 1, `color_mode` (system/light/dark) and optional integer
`density_scale` (−20 through 20), plus optional `palette_id`. Older color-only documents load default density
without rewriting. Invalid or out-of-range density uses System/default density
with visible feedback and preserves the file until an explicit Save. Initial UI starts
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

`tests/appearance.rs` has eight focused cases:

- Defaults and all mode roundtrips/reopen; sibling popup preferences unchanged.
- Invalid JSON, future version, unknown mode and unexpected fields use System
  without rewriting; explicit choice repairs the small file.
- Stale snapshots, oversized/symlink/invalid-directory cases preserve files.
- Older color-only documents preserve their mode and bytes on load, using zero
  density until an explicit save.
- All 41 density choices reopen correctly; color saves retain density and density
  reset retains color. Out-of-range and noninteger JSON values preserve bytes.
- Stale density saves preserve externally changed color/density; reload can then
  reset only density successfully.
- Native palette discovery, stable selected-ID/density persistence, removal and
  restoration, built-in color reset and preservation of external palette files.
- Strict data-only palette parsing, malicious IDs, invalid colors/metadata,
  symlinks, 16 KiB file limit, 32-file catalogue and 64-entry scan limits.

The existing history export integration case now creates actual saved Dark
preferences, refuses this target even with replacement enabled and requires its
bytes unchanged. Use the owned-network wrapper for the full core suite as the
main skill documents.

`python scripts/verify-headless.py --feature appearance` checks actual Kit Settings color/density/palette/reset/reload controls:

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
11. Actual color reset returns to saved System/Light while retaining density.
12. Five density increments preserve copied unsaved rule fields and exact rem/font
    scaling; actual Kit Button bounds differ only within one physical pixel.
13. Density changes on a live permission window retain actual Deny once input.
14. Minimum −20/60% and the disabled smaller control.
15. Maximum 20/140% and the disabled larger control.
16. Density reset restores the original basis without accumulated multiplication.
17. Reload refuses out-of-range density 21 and preserves the original file.
18. An explicit density choice repairs that document.
19. An owned data-only Ocean palette button changes actual native theme tokens.
20. A density change retains the selected native palette on the dashboard.
21. A built-in color choice clears the native palette while retaining density.
22. A fresh GPUI context restores the saved native palette and density.
23. A missing palette file retains the saved ID, uses built-in colors and explains
    the fallback without rewriting preferences.
24. Restoring the owned file and Reload reapplies the native palette. Its catalogue
    bytes remain untouched by preference operations.

The test platform's appearance is fixed Light. This proves System initial/reset
resolution; it does not prove an actual native OS appearance-change callback.
Native compositor/input, packaged startup, all-page contrast, complete custom themes,
whole-table density and broad resource measurement remain separate. Tests never change the
user's real desktop theme or connect to the installed daemon/system socket.

## Language selection proof

This run exposed horizontally clipped language buttons. The language SettingItem
now uses the pinned Kit vertical layout with bounded full-width button wrapping.
The test reaches its group through the real Kit sidebar, using its generated
index-path ID. It records actual control geometry if a button is unreachable.

The appearance/language journeys use Save after each committed selection; a
rejected save retains the previously applied theme. See [Settings](settings.md).

Language persistence and fresh-session locale loading use
`python scripts/verify-headless.py --feature language`.

# Native display density

The current Settings UI stages color, density, palette and language choices until
**Save**. **Close** discards the draft, and failed saves preserve the applied
appearance. Earlier per-click save behavior described in this plan is historical;
see `.agent/skills/verify-rustleblower/features/settings.md` for the current UI contract.

This slice adds a functional native density preference and a bounded native
palette catalogue. It does not complete language, final whole-app layout or exact
Qt Material parity.

The reference is OpenSnitch commit
`a1353848ba1b660320e90cefea782c3fba272c00`, read from the retained bare Git cache
without modifying it. In `ui/opensnitch/res/preferences.ui`, `spinUIDensity`
accepts integers from −20 through 20, default 0. In
`dialogs/preferences/sections/ui.py`, the choice becomes the `density_scale`
theme option. The widgets are hidden for System because upstream delegates the
option to Qt Material. `utils/themes/themes.py` retains the choice independently
of the selected theme and reapplies it on launch.

Rustleblower exposes the same integer choices as a native rem scale, independent
of System/Light/Dark. Zero preserves the existing size; each step changes size by
2%, giving 60% at −20 and 140% at 20. This mapping is a native product choice,
not a claim that Qt Material's pixels use the same formula. Larger numbers make
content larger and less compact. The range is validated before persistence.

## Pinned native component path

GPUI Kit is pinned to 0.7.3, with GPUI Fast 0.1.7. Its component
`Theme::update` synchronizes the Base theme projection and refreshes all windows.
Its `Root` presentation plugin `WindowState::prepare` sets each window's rem
basis from `theme.font_size` before layout. Ordinary Kit buttons use rem sizing
(`h_8`, `px_2p5` for Medium), so density changes their actual height, horizontal
padding and text. Application rem spacing/text changes with the same basis.
Monospace theme font size is scaled as well. The appearance view uses Kit
Settings/SettingPage/SettingGroup/SettingItem and Buttons, following FerryDock's
existing native appearance structure.

The unscaled font baselines are captured once at native appearance initialization.
Every change derives from those values; repeated choices and color changes do
not compound scale. Existing feature entities, filters and editor state survive
the global theme refresh. Newly opened Kit windows get the current rem basis.
There is one rendering backend.

Fixed `px(...)` dimensions do not change: window geometry/minimums, explicit
column widths, icons and fixed-pixel feature elements retain their sizes.
Pinned Kit `Size::table_row_height` and table cell padding return `Pixels`, so
table rows and cell padding do not scale. The table text can scale. This matters
at large settings; broad whole-app overflow/contrast/input assessment remains in
the required final UI pass. It is not whole-table density parity.

## Settings, persistence and ownership

Appearance → Display density has smaller/larger buttons, a numerical
scale/percentage readout, and Default density. Boundary buttons disable at −20
and 20. Default density resets only density; Use System appearance resets only
color. Reload saved appearance restores both choices. Close preserves the
current session selection.

The existing `<database>.appearance.json` version-1 document gains an optional
integer `density_scale`. Old color-only files load with density zero without
rewriting. Existing color-only API saves retain the snapshot's density. The UI
saves both session selections together so changing one cannot reset the other.
Malformed/out-of-range density falls back to System/default density with visible
feedback and no automatic rewrite. An explicit choice can repair a small invalid
document. The 4 KiB file bound, regular-file/symlink checks, content-hash stale
protection, no-clobber first creation and temporary-file publication remain.

All file work runs on the background executor. One application global retains
two small choices, the original font baselines, one snapshot and one load/save
job. Settings controls disable while the job is pending. A failed save keeps the
chosen appearance for the session and explains the failure; another explicit
choice retries. No worker queue, telemetry/history wait, socket access, policy
command or permission-reply dependency was added.

## Verification boundary

`tests/appearance.rs` covers all 41 source density choices, old color-only files,
independent color/reset persistence, malformed/out-of-range values and stale
snapshots preserving external edits, in addition to the prior bounded-file
cases. Test success is reported separately from actual GPU/input evidence.
The owned background appearance journey must prove real control bounds, retained
editor fields, fresh-window initialization, reset/reload and independent prompt
responses before rendering/input completion is claimed. Native compositor,
installed system daemon and packaged startup are separate proof levels.

## Installed native palettes

OpenSnitch's pinned `utils/themes/themes.py::list_themes` discovers the installed
Qt Material catalogue, persists a theme name and applies that file at startup.
Rustleblower provides the equivalent native choice through data-only JSON
palettes installed in `<database>.themes/`. The palette filename without `.json`
is its stable ID: 1–64 ASCII letters/digits/hyphens/underscores. Names allow
1–80 UTF-8 bytes and reject control characters. This is a native format; Qt XML,
Qt Material stylesheets and executable plugins are not imported or executed.

For a database `/path/history.sqlite3`, an example
`/path/history.sqlite3.themes/ocean.json` is:

```json
{
  "version": 1,
  "name": "Ocean",
  "mode": "dark",
  "colors": {
    "background": "#14212b",
    "foreground": "#e2edf4",
    "primary": "#65c4ed",
    "primary_foreground": "#092635",
    "secondary": "#283f50",
    "secondary_foreground": "#e2edf4",
    "muted": "#243744",
    "muted_foreground": "#b0c6d5",
    "accent": "#244f64",
    "accent_foreground": "#e2edf4",
    "border": "#476173",
    "danger": "#d95167",
    "danger_foreground": "#ffffff"
  }
}
```

All 13 colors are required opaque `#RRGGBB` values; extra fields are rejected.
Mode must be Light or Dark. The palette overrides Kit's corresponding native
color fields after restoring that built-in mode, using the same `Theme::update`
path as FerryDock's native palette. Popover/background text and focus ring follow
the palette. Other Kit semantic fields retain defaults from the declared mode;
this limited palette format does not cover every Kit theme token, fonts, layout,
highlighting or window geometry. Contrast remains part of the final UI audit.

Discovery runs only at initial load or explicit Reload, on the same background
worker as appearance persistence. It accepts at most 32 JSON files, at most
16 KiB each, and scans at most 64 directory entries without recursion. Oversized
catalogues reject the whole scan with visible feedback; invalid, unreadable,
nonregular and symlink palette files are skipped with a bounded count warning.
The catalogue directory must itself be a regular directory rather than a symlink.
No file watcher, network requests or automatic retries are installed.

Each discovered palette has an actual Kit button in Appearance. Choosing it
changes the native colors immediately and saves its ID together with mode and
density. Selecting System/Light/Dark returns to the standard palette and clears
the saved ID. Density changes and density reset retain the selected palette.
`palette_id` is optional in existing version-1 appearance documents.

If a saved ID is missing/invalid/unreadable, loading retains that ID, preserves
density, applies the saved built-in color mode, and reports visible fallback.
Installing/restoring its file then Reload can restore the palette. Density
changes retain an unavailable ID; an explicit built-in color choice clears it.
Catalogue data is a bounded snapshot until Reload: changing/removing a file does
not silently change the current session. Palette files are never modified, so a
stale catalogue cannot overwrite an external palette edit. Appearance preference
publication retains its existing content-hash stale protection.

Two additional core cases cover discovery/selected-ID reopen, old API retention,
palette removal/restoration, color reset preserving density, strict colors and
metadata, malicious ID rejection, extra executable fields, file/directory bounds
and symlink preservation. Core/build results and actual palette GPU/input proof
must be reported separately; prior color/density screenshots cannot establish
new native palette behavior.

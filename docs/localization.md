# Native language catalogue and migration

The source contract is pinned OpenSnitch
`a1353848ba1b660320e90cefea782c3fba272c00`. Its
`ui/opensnitch/utils/languages.py` enumerates installed locale directories, keeps
English as fallback, uses `QLocale.system()` for the system choice and loads a
selected `.qm` translator. `ui/opensnitch/dialogs/preferences/settings.py`
marks a language change as requiring restart. Source has English plus 25 locale
catalogues: ar, cs_CZ, de_DE, es_ES, eu_ES, fi_FI, fr_FR, he_IL, hi_IN, hu_HU,
id_ID, it_IT, ja_JP, lt_LT, nb_NO, nl_NL, pt_BR, ro_RO, ru_RU, sq_AL, sv_SE,
tr_TR, uk_UA, zh_Hans, zh_TW. Installed coverage varies; merely listing these
names would not establish language parity.

Rustleblower's native data format is original MIT code and original native
translations. No OpenSnitch GPL `.ts`, `.qm` or translated message assets are
copied. English Kit message names/text describe the API of the existing MIT
GPUI Component dependency; French Kit messages are newly written here. This
format does not load Qt translators or executable translation plugins.

## Current bundled catalogues

The 2026-10-17 completion checkpoint covers the five existing bundled languages:
English, French, German, Spanish and Japanese. Each covers the full English-schema message catalogue; the inventory records
the current exact count. Three Luna subagents filled the 1,438 missing messages per language
for German, Spanish and Japanese (4,314 additions), preserving all 934 existing
translations in each. Integration added the new Settings label “Table highlighting”
in all five catalogues, for 1,439 additions in each of DE/ES/JA (4,317 total).
Later browsing-filter and Settings integration added validation and Settings
messages in all five catalogues. No OpenSnitch translation assets or translation services were used. The other 21 source languages remain outside this requested scope.

`scripts/i18n-inventory.py --check` now requires complete key coverage and exact
placeholder sets in every shipped catalogue, while externally installed partial
catalogues retain their supported English fallback. The existing 512 KiB/4,096-key
catalogue limits and English-plus-active runtime map are unchanged. Historical
partial-catalogue checkpoints below retain their original scope; this completion
checkpoint supersedes their coverage counts.

### Completion verification

Evidence lives under `.verification/translations/`. The final inventory requires
complete shipped coverage and zero direct UI literal candidates; all five
catalogues pass with no missing/unknown keys or placeholder mismatches. The final
catalogue review records byte sizes/hashes and preservation of the original 934
DE/ES/JA messages. All catalogues remain below 512 KiB, all values below 2,048
UTF-8 bytes, and the runtime retains only English plus the active message map.

The normal desktop build (`python scripts/cargo.py build --locked`) passes.
The final native `tests/i18n.rs` run passes eight tests, including every DE/ES/JA
message, regional aliases, external partial fallback, hostile input/bounds and
the 116 public Kit messages in all five languages. An earlier final core-only
run passed seven tests before the one Settings-label addition. The eleven-frame
background GPU/input journey at
`.verification/headless/20261017T053927Z-9be33d16/result.json` proves saved language
selection and fresh GPUI startup for French/German/Spanish/Japanese, rendered
Japanese glyphs, retained unsaved drafts and translated typed filters. It also
verified the full-width vertical language-control layout that fixed clipped
buttons. Its source predates the small final German/Japanese terminology fixes,
the Settings label addition and the concurrent consolidated Settings refactor;
those later Settings navigation/layout changes need their own UI proof. This is
sampled native rendering, not a native-speaker review of every translation or a
package/real-daemon/OS-process-restart claim. No system socket, daemon or firewall
policy was changed.

## Phase 1 API and bounds

`src/i18n.rs` provides `LocaleId`, `LanguageChoice`, `Catalogue`, `Localizer`,
`text(key)` and `format(key, &[(name,value)])`. Locale IDs contain at most 32
ASCII bytes in bounded language/script/region subtags; underscores normalize to
hyphens. `system` is reserved for the preference. `system_locale` resolves the
first nonempty `LC_ALL`, `LC_MESSAGES`, `LANG`; encoding/modifier are removed,
C/POSIX/invalid input uses English. Resolution tries exact locale, successively
shorter subtags, then English. `fr_CA.UTF-8` consequently resolves to the
bundled `fr` catalogue. The same parent fallback resolves `de-DE` to `de`,
`es-MX` to `es` and `ja-JP` to `ja`; other region/script aliases are not guessed. Unknown selected locales remain saved with a visible
English fallback warning.

The English schema and French catalogue are compiled into the binary.
Additional installed catalogues live in `<database>.languages/`, derived from
the existing appearance store path. Discovery and loading are explicit
background operations, with these limits:

- 64 directory entries, 32 JSON files, no recursion or symlink directories/files.
- 512 KiB per file, at most 4096 unique message keys, key length 96 ASCII bytes.
- Native language name at most 80 UTF-8 bytes; message value at most 2048 bytes.
- The file stem must normalize to its declared locale. Duplicate locales,
  unknown keys, duplicate JSON message keys, invalid controls and malformed or
  mismatched named placeholders are rejected. Bundled English/French cannot be
  overridden by an installed duplicate.
- Discovery retains descriptors, counts and SHA-256, rather than every message
  body. Activation reopens only the selected catalogue and requires its hash to
  match discovery; deletion/change produces English fallback and a warning.
  Reload creates a fresh catalogue snapshot.
- Runtime keeps English plus the active message map. Missing translated keys
  fall back individually to English. An unknown program key is displayed as its
  bounded ID. Text is plain text; no HTML, code or SQL interpretation occurs.
- Formatting accepts at most 16 distinct named arguments, each 4096 bytes;
  argument names must exactly match the message placeholder set. Output is
  bounded to 16 KiB. Values containing `%{...}` are inserted once and never
  reinterpreted. Use named values for runtime validation errors, counts and
  filenames; never translate protocol values, identifiers or process data.

Example installed partial catalogue (`de.json`):

```json
{"version":1,"locale":"de","name":"Deutsch","messages":{"language.title":"Sprache"}}
```

Partial installed coverage is reported by `Language.translated/total` and uses
English for absent keys. The compiled EN/FR phase 1 baseline had 124
messages each: 116 pinned Kit messages plus 8 language preference messages.
The ongoing phase 2 migration adds keys; the inventory records the current exact
count. Neither count represents full application coverage.

`install(Localizer)` updates the in-memory global. The pinned 0.7.3 Kit exports
`gpui_kit::component::_rust_i18n_extend`; a `Once` registers a `rust-i18n`
4.2.4 backend under the `gpui_component` namespace. The direct dependency uses
the version already in Cargo.lock. It translates Kit's own menus/placeholders
through the original catalogue without vendoring Kit, installing another
renderer or adding gettext. A desktop integration test checks all 116 French
messages through Kit's actual public translation function, including named
placeholders. Existing Kit locale falls back through the same English schema.

## Saved selection and session behavior

The existing version-1, 4 KiB appearance JSON gains an optional `language`
string; omission means `system`. All existing appearance save APIs preserve it,
and language saves preserve color mode, custom palette and density. The same
single background job, stale-file hash checks and no-clobber initial creation
apply. Saving does not change the active session locale or editor values.
System and discovered native-name buttons appear in the final Language group;
selection reports that restart is required. Reload discovers catalogue changes
and reloads the saved choice while retaining the active session language.
A missing catalogue/invalid preference is never automatically rewritten.

At initial startup, English is immediately available. The appearance worker
loads the saved language and catalogue asynchronously, then installs it once.
No permission admission/reply awaits catalogue I/O. Phase 2 updates strings
captured at entity construction when that initial install refreshes windows,
without resetting user input; ongoing user selections still apply at restart.
The first migrated inputs register bounded weak references (at most 512 live
inputs); startup refresh calls Kit `set_placeholder` and preserves draft values.
Localized Select items retain typed values and compute their title at render time.
Further semantic coverage review remains in progress; the scoped French input proof is recorded below.

## Current English/French presentation migration

The current schema contains more than 2,200 original English/French messages,
including the pinned Kit's 116 messages, typed selector titles, all current
native windows and field descriptions, staged reviews, status text and reviewed
core diagnostics. `scripts/i18n-inventory.py` is the exact current count and
placeholder gate; feature work can add entries after this paragraph is written.
The direct-literal gate also inspects DescriptionList items and accessibility
labels. The conventional `argv[index]` argument index remains technical notation.

At initial locale installation, a registry of at most 64 live native window
handles refreshes only their titles through public `Window::set_window_title`.
Closed handles are pruned. The existing 512-input registry refreshes only
placeholders; typed Select titles are computed at render. Neither path resets
input values, staged reviews or a captured permission fallback. A later saved
language choice still applies at restart.

Core/control strings remain stable. Presentation translates exact reviewed
static messages and 19 explicitly registered dynamic producer contracts with a
single named capture. Captures are nonempty and at most 4096 bytes, complete
messages at most 16KiB; count captures require at most 20 decimal digits. Matching
is anchored to fixed English prefixes/suffixes, never learned from an installed
catalogue, and captured paths/details are inserted as plain text without
recursive translation. Unknown or out-of-bounds diagnostics remain verbatim
English. Separate `ensure!` and metadata/format review queues prevent the direct
button scan from being mistaken for total semantic coverage.

Fixed source-template formatting also has bounded English fallback. When a
new `tr_args` source has no catalogue entry, the shared one-pass formatter
requires an exact named argument set and formats the English source; inserted
values are never recursively interpreted. A known-source argument mismatch,
invalid template, duplicate argument, value above 4 KiB or output above 16 KiB
keeps the original source visibly unchanged. Key-based lookup remains separate.
The inventory still requires every recognized presentation source to be
catalogued, including literal arms of conditional `tr`/`tr_args` calls.

Notification titles and action labels translate on the existing notification
worker; freedesktop action IDs remain stable. Daemon summaries, URLs, reported
values and user-authored missed-popup message templates are preserved. Complete
connection-detail clipboard export translates headings separately, preserving
large captured command lines rather than passing payloads through bounded UI
format arguments.

Source locale counts, without copied source messages or translations, are in
`.verification/i18n-upstream-coverage.json`:25 TS catalogues range from zero
finished/nonempty messages to100% of their active source messages. This inventory
counts unfinished/empty entries as untranslated and excludes obsolete/vanished
entries; it is source evidence, not a claim about a new native catalogue.
The pinned `ui/i18n/generate_i18n.sh` invokes `lrelease` without
`-nounfinished`; unfinished entries with nonempty translations therefore must
also be considered when reviewing the installed source language scope.
`.verification/i18n-upstream-release-coverage.json` separates both counts:
Japanese has 1 finished plus 257 nonempty unfinished entries, German 419 plus
26, and Spanish 509 plus 15. These are source counts, rather than an executed
Qt translator proof. Source context counts are retained separately in
`.verification/i18n-upstream-context-coverage.json`; no upstream translated
catalogue contents are copied into native assets.

## Coverage gates and next phases

Run `python scripts/i18n-inventory.py --check --output <evidence.json>`.
This checks complete shipped catalogue key and placeholder coverage and every
statically recognized `i18n::text/format` key reference. The JSON also reports
all source literal counts and conservative direct UI literal candidates with
file/line/API. `--require-complete` additionally fails while direct UI candidates
remain. It is a migration aid, not a claim that a regex classifies every
semantic user-facing string: enum labels, dynamic messages, core validators,
notifications, window titles and accessibility labels require manual inventory
review. Phase 1 reports 621 direct UI literal candidates and leaves them visible.

Phase 2 must migrate all current English/French surfaces, including features
added by other agents, plus shared core error messages and notification text.
Stable typed option values must be separated from localized display titles
(`SearchableListItem::value/title`) before translating Select labels. Tests must
verify that selecting translated options still produces unchanged protocol
predicates, preserve staged inputs during the async initial install and prove
restart/reload/fallback through actual input. No raw wire value or path belongs
in a translation key.

Phase 3 remains part of the full parity goal: original catalogues for the other
source locales with actual measured coverage/fallback and suitable native fonts,
RTL direction/layout and CJK proof. The bounded bundled registry now includes
original German, Spanish and Japanese partial catalogues with 934 entries each,
including all 116 pinned Kit messages. English and French remain mandatory
complete catalogues; partial catalogues reject unknown keys and placeholder
mismatches while deliberately omitting untranslated entries. Missing messages
use the existing English fallback, without English filler to inflate coverage.
The appearance picker displays each catalogue's actual entry count against the
current compiled English schema, plus active catalogue coverage after regional
resolution. A missing or invalid selection activates English and reports English
coverage, rather than the saved ID's stale count. Saving still requires restart.

The current schema contains more than 2,300 messages, including diagnostics and
native features absent from the source's 649-message catalogue. Raw counts or
percentages therefore do not establish source-equivalent coverage. The staged
surface review in `.verification/i18n-phase3-candidates/surface-scope.json`
tracks actual recognized native UI messages separately. Appearance/language,
permission-popup and dashboard static uses are covered in the three partial
catalogues; rule/history/subscription and dynamic metadata/error coverage still
requires review. Source nonempty message counterparts are only a review floor,
not a semantic parity claim. Other source languages, actual selection/restart,
RTL layout and native CJK glyph proof remain open.

Phase 3 focused core 6 cases and native 7 cases pass, with the pinned public Kit
extension returning all 116 expected messages across EN/FR/DE/ES/JA. Native
all-features/all-targets check also passes. Scoped logs and exact owned source
hashes are retained in `.verification/i18n-phase3/result.json`; later catalogue
additions have their own inventory/source checkpoint. These checks are separate
from native GPU/input and font rendering evidence.

## Verification

Scoped phase 1 logs and coverage inventory are under
`.verification/i18n-phase1/`. Focused core tests cover locale resolution,
placeholder bounds/injection-as-text, invalid/duplicate/oversized/symlink
catalogues, partial English fallback, catalogue stale-file rejection and saved
appearance preservation/stale-file protection. Desktop compilation and the
116-key Kit integration test are separate from GPU/user-input and real-daemon
proof. The focused eight-frame `--feature language` GPU journey passes at
`.verification/headless/20261017T030406Z-cd9a6a61/result.json`: actual French
choice/save without changing the active session or unsaved editor; fresh GPUI
startup activation including Kit Copy; translated typed rule/history fields
producing exact rows; missing saved language fallback without file rewrite and
explicit System repair. All PNGs and owned socket/process cleanup pass. The
aggregate `.verification/language-result.json` retains the exact test function
and boundaries. The current subsection4 rerun retains a complete 116-file source checkpoint at
`.verification/language-final-source/manifest.json`. Applied summaries still
contain English comparison names and Settings long-label clipping remains;
later phase2 work is separate.
Fresh GPUI startup is not an OS process relaunch or complete language parity.

# Language and native translation catalogue

Scope: implemented bounded native catalogue and restart-required selection;
English/French application migration is in progress. Do not mark full language
parity complete from a picker or translated Kit placeholders alone. Read
[localization architecture and source bounds](../../../../docs/localization.md).

- `src/i18n.rs`: validated locale/choice, POSIX System resolution, installed
  descriptors, English fallback, named placeholders, in-memory runtime and the
  pinned Kit public backend extension. `locales/en.json` and `fr.json` contain
  original native messages; no upstream GPL translation asset is reused.
- `src/appearance.rs`: optional language string in the existing bounded file,
  old-file defaults and same stale/no-clobber protection. Saving language
  preserves color/density/palette and other appearance saves preserve language.
- `src/app/appearance.rs`: actual System/discovered native-name choices, shared
  background worker, retained current session and restart-required message;
  async initial install never gates a permission reply.
- `LocalizedOption<T>` separates typed Select values from display titles.
  History/rule condition fields and comparisons, and history export format, use
  this form. Startup refresh changes input placeholders without replacing values.

Focused pure tests: `python scripts/cargo.py test --no-default-features --locked
--test i18n --test appearance` (12 cases). Desktop `--test i18n` adds the actual
116-key pinned Kit extension proof; it requires neither GPU nor a native window.
The same desktop test verifies French typed option titles with unchanged enum
values. Catalogue tests include partial fallback, unavailable/stale selected
files, symlinks, oversize/duplicate/unknown messages, wrong placeholders, literal
argument insertion and appearance save/reopen/external-file protection.

Run `python scripts/i18n-inventory.py --check --output <evidence.json>` and retain
its exact key counts, recognized call sites and remaining literal candidates.
`--require-complete` is expected to fail during migration. A clean static-key
check proves the recognized catalogue references; it does not prove complete
user-facing coverage or actual input/layout.

Required focused GPU journey (queued with the verification agent): save French
through `appearance-language-fr`, retain draft/density/palette and active session
language, independently reopen the JSON; start a new app with saved French;
check translated navigation and actual French history/rule Select choices still
produce the exact expected predicates; exercise named messages, reload and
unavailable saved-locale English fallback. Current observed appearance host is
`appearance-content`; language group follows custom palettes and may require
scrolling. IDs use canonical locale suffixes, not translated names.

Broader unfinished proof: all current application text in French/English,
startup-captured window titles/labels, permission input while catalogue work is
held, source-equivalent coverage of the remaining 25 locales, RTL/CJK fonts and
layout, and the final whole-app UI/UX consistency pass. No real-daemon language
semantics are inferred from build/core/Kit results.

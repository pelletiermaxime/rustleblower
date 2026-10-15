# Application rules

The Application rules tab allows manual recovery after a missed prompt. Catalogue
data lives in SQLite; changes go through the existing daemon notification stream.

## Sub-features and entry points

- `rules-browse`: Application rules tab → filter name/executable/description;
  All/Permanent/Temporary; Next/First pages of at most 200 rules.
- `rules-action`: select a rule → Allow/Deny/Reject; retain its operator, duration,
  priority and logging settings. Wait for daemon confirmation before updating.
- `rules-enabled`: select a rule → Disable/Enable.
- `rules-create`: New application rule → absolute executable path, optional name
  → Allow always/Deny always. The match targets that executable.
  Creation and editor renaming reject names already present in the confirmed
  catalogue. Import's explicit Replace option remains the overwrite workflow.
- `rules-delete`: select a rule → Delete → Keep rule or Delete rule.
- `rules-editor`: select → Edit…; or New application rule → All rule options….
  Edit metadata, action, preset/custom duration, priority, logging and a flat AND
  group of conditions. Match fields include all installed 1.7.2 operands;
  use Equals/Regular expression/CIDR/watched-directory as appropriate. Match JSON
  roundtrips the operator. Save waits for confirmation. A rename confirms the new
  rule before deleting the old one; partial failure remains visible.
- `rules-duplicate`: select → Duplicate… → change the draft → Save rule. Cancel
  sends no command. Saving retains the original rule.
- `rules-transfer`: Import… → JSON file/directory path or Read clipboard → review
  → Apply reviewed rules. Duplicate names skip by default; the Replace checkbox
  permits overwriting. Export filter… includes all pages; Export selected… uses
  checked names or the highlighted rule. Choose JSON file/folder/clipboard.
- `rules-bulk`: check rows/Select page → action → review → Apply. Names remain
  selected while paging. Clear selection, or change the filter, clears them.
  The review can include every matching rule across all pages. Stop waits for the
  current reply; retry sends only remaining rules. Delete has a review step.
- `rules-errors`: daemon error preserves the old catalogue value; timeouts and
  disconnects explain that confirmation is unknown and require a reload.
  A daemon ERROR can follow an in-memory change before a disk failure, so a
  persistent warning identifies cached entries as the last confirmed values.
  A fresh inventory clears that warning; do not claim rejected commands prove
  unchanged live policy.
- `rules-popup-observation`: lasting popup replies appear labelled awaiting
  observation with actions disabled. A daemon Ping containing that rule confirms
  its reported values; self-generated decision history does not. Exports include
  proposals, but mutation reviews/current-rule editing require confirmed entries.
- `rules-refresh`: Refresh connection → request a fresh Subscribe inventory.
  Refuses pending decisions/commands and keeps the owned socket inode unchanged.
  Core protocol proof covers guards and replacement; native refresh input remains
  unverified. This deliberately reconnects the notification stream.
- `rules-bounds`: 10,000 rules / 2 MiB per inventory, 64 KiB encoded per rule,
  16 pending commands / 8 outbound queued messages. Current inventory readiness
  and session identity gate edits; stale replies cannot modify a replacement.

## Verification

Run `python scripts/cargo.py test --no-default-features --locked`. The rule protocol
tests exercise a real UI-side gRPC server, daemon command/reply streams, catalogue
queries and SQLite durability. They cover action rejection/confirmation, preserved
operators, enable/disable/delete and reconnect cancellation. Unit tests check
pending-operation capacity, expiry, inventory readiness and filename validation.

Run `python scripts/verify-headless.py`. The expanded flow uses actual Kit tab,
ListItem, Input and Button events against an isolated gRPC command fixture:

1. Open Application rules; inspect `08-application-rules.png`.
2. Next/First; select a rule and allow it; inspect `09-rule-allowed.png`.
3. Create a permanent rule; Ctrl+F and type its executable; inspect `10-rule-created.png`.
4. Reject a daemon command, require the old action and visible error;
   inspect `11-rule-rejected.png`.
5. Cancel deletion, then confirm it; inspect `12-rule-deleted.png` and the empty page.
6. Edit a source rule, rename, set description/priority/logging/duration, and add
   a case-insensitive host regex; inspect `13-rule-editor.png` and
   `14-rule-edited-draft.png`. Require acknowledged ChangeRule then DeleteRule,
   the updated persisted values and editor closure/focus restoration.
7. Cancel a duplicate (no command), reopen it, roundtrip Match JSON, and try an
   invalid custom duration (no command). Enter `90s` and save. Require two rules,
   original unchanged and copied operators equal; inspect `15-rule-duplicate.png`,
   `16-rule-match-json.png`, `17-rule-invalid-duration.png`.
8. Export the filter to file/clipboard; import that file with duplicate skipping;
   import adjacent JSON from the clipboard, fail the second command, then retry
   only the remaining rule. Inspect `18-rules-export.png`, `19-import-skipped.png`,
   `20-import-review.png`, `21-import-partial-failure.png`.
9. Mark a row/Select page; review Disable; hold its daemon reply and answer a
   separate permission popup, then Stop and resume. Review Delete, close it
   without commands, reopen and confirm it. Inspect `22-bulk-selection.png`,
   `23-bulk-stopped.png`, `24-bulk-delete-review.png`, `25-prompt-during-bulk.png`.
10. Explicitly replace an existing name. Select 201 names across two pages and
    export exactly that subset. Then apply Disable to all 229 remaining matches
    and require later-page persistence; inspect `26-bulk-all-pages.png`.
11. Answer a permanent popup; inspect `37-popup-rule-awaiting.png` for its label
    and disabled edits. Report that rule through the fixture's Ping and require
    the daemon-reported metadata plus `38-popup-rule-observed.png`.

`tests/rule_transfer.rs` and the snapshot unit test separately verify 260-rule
file/folder/clipboard roundtrips, filtering/selection, snapshot consistency under
concurrent writes, filename suffix collisions, existing destination preservation,
JSON/object/count limits and import formats. Native platform file pickers are
not yet input-tested; the GPU journey types paths and uses the real clipboard API.

The Python demo fixture supplies six application rules by default (`--rules N`
changes that) and processes commands in memory. `--reject-rule-changes` enables
the error path. It never changes real firewall rules. Existing native history
and prompt checks still use owned instances, but rules input itself currently
has background GPU proof, not compositor/native-input proof.

## Remaining parity work

Richer keyboard navigation and per-node targeting remain tracked
in `docs/port-plan.md`. Do not imply those are implemented. Confirmed daemon
responses prove the protocol outcome, not independent kernel enforcement.
The history feature covers creation from a recorded executable/destination and
current matched-rule lookup, including existing-name protection in both editors.
Background input covers representative editor fields; CIDR/list/environment/
checksum and every field variant still need dedicated input proof. Core JSON
tests cover timestamps, legacy list encoding, omitted protobuf flags, safe
field/type pairs and custom duration units. The original daemon's flat AND
serialization limitation is preserved explicitly; nested groups cannot be saved.

# Subscription collections: source contract and remaining UI work

Reference: OpenSnitch a1353848, source version 1.9.0. The retained
`ui/controllers/rules_editor_controller.py::configure_rules_dialog_for_local_user` restricts
subscription-generated rules to the local node and current UID. This boundary
was missing from the first Rustleblower template and is now corrected for new
single-list and collection drafts. Existing daemon rules are never silently
rewritten. The generic editor still lets an explicit user edit all conditions.
`rustix::process::getuid` obtains the real UID; the already locked rustix 1.1.5
is now a direct dependency with its process feature. No new package is bundled.

## Implemented backend foundation

`subscriptions/collections.rs` materializes typed group/all directories. Source
files remain under `sources/<id>/<kind>` and are retained independently. Active
membership requires an enabled, nonremoved subscription and its last valid
publication of the selected reader kind. A changed URL keeps compatible last-good
data; changing kind removes the previous kind from active collections while
retaining its source file. Group slugs follow upstream lowercase/ASCII separator
rules; reserved `all`, empty and duplicate labels do not make extra groups.

The five all-list directories always exist. Group directories exist only for
active sources, separated by daemon reader kind. Every directory contains at most
128 source symlinks and one comment-only refresh marker. The total source-link
bound is 128 subscriptions × (one all + eight groups) = 1152. The collection query
has at most 64 visible rows; search/paging cover the complete bounded catalogue.

Each rebuild constructs an owned generation and atomically replaces one `current`
symlink. No system directory, daemon configuration or rule definition is changed.
Only app-marked prior generations are removed; directory symlinks, unexpected
pointer files and unmarked debris are refused. A bounded startup sweep recovers
marked orphans. Errors after saved metadata/published data surface uncertain
collection state and stop this feature worker while gRPC/permissions remain
independent. This is not a filesystem/SQLite crash transaction.

The pinned daemon misses membership replacements when visible filenames change
but their total count stays equal. A stable `0-refresh.txt` containing only a
comment forces a new mtime on changed generations. Its timestamp advances at
least one second, including on coarse filesystems. A persisted publication epoch
also invalidates the generation signature when payload size and source mtime are
unchanged. Metadata-only renames do not rebuild unchanged collections. The Go
reader/monitor harness now includes equal-count replacement proof.

`Request::Collections` exposes a bounded query to the native app; collection
rule drafts share the exact UID-AND-directory factory with single subscriptions.
The native group/all picker now uses these entry points. Current single-list
rules still point directly to retained data; collection activation does not
implicitly alter those definitions. Group/all rules use the active collections.

## Native UI implementation and verification

1. **Implemented:** Rule collections is an explicit Blocklists action and
   Ctrl+Shift+L shortcut. Empty setup displays five zero-member kinds. Search,
   Group/Type/Active sources sorting, descending direction and 64-row pages act
   on the complete matching catalogue.
2. **Implemented:** direct typed target resolution on the subscription worker
   precedes review. The captured local node and UID-AND-directory draft use the
   existing editor/ACK/cache/session guards. Cancel sends no policy command.
3. **Implemented:** associated-rule inspection uses an explicit collection or
   single-subscription target, independent of row selection and current global
   node during pagination. It scans all confirmed catalogue pages, retains
   16 rules/512 KiB, and excludes proposals. An edit rereads the confirmed rule;
   a changed/deleted snapshot is refused until Refresh snapshot. The callback
   rechecks its captured session and the single-editor guard.
4. **Implemented:** enabled/group/input-format changes and restoration of saved
   subscriptions require a before/after membership review even if a download
   completes while the form is open. Back preserves the form; Cancel preserves
   saved data. Removal confirmation distinguishes collection membership from
   retained direct-list data. Automatic-update Pause remains separate.
5. **Verified:** actual Kit input on empty/all/group views, global query/sorts,
   86 collections across pages, UID/path rule creation/cancel/ACK, associated
   group rules, concurrent confirmed changes, snapshot refusal/refresh and
   membership review/back/cancel/apply. Native empty-page/review/search proof is
   separate from native membership application, rule saving and paging coverage.

A bounded completion guard clears busy state if an asynchronous lookup is
interrupted with its original window. The ordinary input journey verifies normal
completion; explicit in-flight window-close/reopen proof is still required for
that lifecycle edge. The final whole-app UI/UX pass also remains open.

Reviewed bulk configuration and bounded native/OpenSnitch action/config transfer
are now implemented; see the current subscription feature map for evidence.
Complete-catalogue replacement, duplicate display identities and stable per-ID
extra fields are now implemented, with conservative legacy metadata archives.
Independent count/unit inheritance now covers individual/default and bulk forms,
with frozen-default effective reviews. Notifications, retained-format inspection/cleanup
and complete crash recovery still remain, along with external HTTPS and broad
native/resource/package proof. The metadata archive inspection/manual-resolution
interface is a Rustleblower recovery improvement, not a source-backed upstream
page required for feature parity; retain its records and reimport recovery.
The source-backed pre-save Test URL action is now implemented; see the current
subscription feature map and `.verification/subscription-url-test-result.json`.
Port ranges, the remaining full UI parity, final coherent UI/UX phase and the
requested upstream/Little Snitch research stay in the active goal.

## Evidence

See `.verification/subscription-collections-result.json`. The complete core suite
passes 122 tests, including 12 subscription service cases. The kernel-UID probe
also passes outside the fixture user namespace with real UID 1000. The owned
subscription GPU journey now has 32 frames. The pinned Go harness passes seven tests,
including its real four-second monitor and equal-count replacement marker.
The current UI proof is recorded separately in `.verification/collection-ui-result.json`.
It does not prove every native mutation/paging path, hosted CI, kernel enforcement
or lower RSS. Earlier evidence keeps its original source scope.

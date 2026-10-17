# Subscription publication recovery

The subscription worker previously renamed a validated download into its stable
source filename before committing SQLite metadata. A failed transaction or a
process stop between those steps could leave new bytes under old size/validator
metadata. Normal SQL rollback tests did not prove this boundary.

The native worker now uses one durable publication intent. Its
existing single-download admission means at most one intent and backup exist:

1. Validate the owned source directory and regular destination. Retain an
   existing destination through a hard link in the manager's temporary directory,
   sync the retained inode/directory and commit the intent to SQLite.
2. Rename the already synced validated download to the stable destination and
   sync its parent directory.
3. Commit publication metadata, subscription status/bytes/validators, collection
   epoch, bounded success log and intent removal in one SQLite transaction.
4. Remove the retained hard link, rebuild the existing collection generation and
   drain a durable obsolete-filename queue. Paths still referenced by another
   publication or subscription are preserved. At most four cleanup paths are
   admitted; a failed cleanup stops the worker before another download. Startup
   rebuilds the committed collection generation before retrying cleanup.

An ordinary filesystem/SQL failure restores the previous destination (or removes
an uncommitted first publication) and records a failed refresh. A rollback failure
stops the worker with its durable intent intact. Startup recovers the intent
before temporary-download cleanup or collection synchronization.

Recovery preserves the backup until rollback and intent removal are durable, so
another interruption during recovery can be retried. A committed publication has
no intent: startup retains its new data and removes a leftover backup. Changed
revisions/paths, missing required backups, symlinks and special files refuse
recovery and preserve evidence instead of touching a guessed destination.

The hard link does not copy the old list. Physical storage can still include the
one in-flight download in addition to retained published bytes; the download's
existing size cap applies. A later retained-file/disk-budget workload test must
measure that peak and obsolete-file recovery separately. SQLite and filesystem
updates are not one externally observable transaction: a daemon reading the
stable file during publication can see validated new bytes before metadata
commit. This journal provides durable recovery and last-good restoration, not an
atomic snapshot across a running external daemon and SQLite.

Implementation is in `src/subscriptions/store/publication.rs`. Five fault/reopen cases pass in
`.verification/subscription-publication-faults2.log`: late SQL/log failure, first
publication rollback, pre/post-rename interruptions and repeated rollback,
committed metadata with leftover backup, unsafe recovery paths, obsolete-name
cleanup before/after unlink, and retained previous-format publications. These
are deliberate fault/reopen simulations and real SQL-trigger failures, not
physical power-loss tests. All 42 cases in the six impacted subscription/history
integration binaries pass with owned loopback fixtures in
`.verification/subscription-publication-regressions.log`. Scoped hashes and
boundaries are in `.verification/subscription-publication-result.json`.
The current 61-frame subscription GPU regression also passes against an immutable
build containing exactly the four journal production/test files from this core
checkpoint (`.verification/subscription-publication-gpu-result.json`). It covers
import/export, stale revisions, bulk defaults, collection changes, held policy
acknowledgements and independent permission input. A later backend change adds
the disabled reputation worker; that lifecycle delta has its own core proof. Whole physical
disk/RSS/concurrent workload proof remains open.

## Manager lock lifecycle

A later complete-core checkpoint exposed an immediate reopen refusal while
other cases could launch child processes. The exact child responsible was not
traced. Linux file locks belong to the
open file description: a child can briefly inherit a CLOEXEC descriptor before
exec, retaining the departed manager's lock after its own File closes. A focused
`try_clone()` reproduction retains the same kernel description without process
timing, fails before the repair, and also proves a live manager remains exclusive.

The private manager-lock guard now explicitly unlocks when its logical owner
drops, then closes its descriptor. The guard drops after SQLite closes and also
covers failures while opening/recovering the store. A descriptor retained by an
old child cannot unlock a subsequently opened manager. The six publication cases
including this reproduction pass in
`.verification/subscription-lock-inheritance-after.log`; that focused pass
predates only moving guard construction earlier to cover initialization errors.
The complete-core retry passes263 cases, including that narrow refinement, with normal parallel test execution (`.verification/whole-core-retry-result.json`). No bounded
wait/retry, test serialization or weakened ownership check masks the race.

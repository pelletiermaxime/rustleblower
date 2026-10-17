# Executable checksum changes

A permission popup with valid reported MD5/SHA1 checksums compares enabled saved
rules for the same executable. A mismatch offers **Review saved checksums…** in
a separate native window. Permission actions and deadlines remain independent.

## Sub-features

- `checksum-warning`: show mismatch presence or catalogue unavailability.
- `checksum-review`: inspect affected rule names/actions/durations and saved /
  reported hashes, with First page / reload and Next page.
- `checksum-selected`: confirm or cancel updating the selected rule.
- `checksum-all`: confirm updates across all affected pages, one command at a time.
- `checksum-stop`: prevent unsent updates, waiting for an in-flight result.
- `checksum-failure`: show confirmed count and daemon uncertainty; fresh inventory
  and a new review are required after errors or session replacement.
- `checksum-isolation`: answer a permission while a rule acknowledgement is held.

## Launch and drive

Use `python scripts/verify-headless.py --feature checksums` in the background.
It creates 35 confirmed rules with MD5/SHA1 and unrelated conditions, opens the
production warning/review, pages, cancels and confirms selected replacement,
holds an ACK and requires unchanged cache plus immediate Allow forever, stops all
after one sent command, exercises a daemon ERROR, then resubscribes with a fresh
inventory and updates the remaining 33 rules across pages. It checks every saved
rule against the metadata-preserving expected replacement and clears the warning.

Require nine JSON checkpoints, `real_gpu: false`, scoped socket cleanup, actual wire/cache assertions
and a passed result. Frames 1–4 show warning/review/next-page/confirmation; 5–7
show waiting, stopped and error states; 8–9 show all confirmed and cleared warning.
Stable IDs are `prompt-checksums`, `checksum-next`, `checksum-reload`,
`checksum-update-selected`, `checksum-update-all`, `checksum-confirm`,
`checksum-cancel`, `checksum-stop` and `checksum-close`.

Use `python scripts/cargo.py test --no-default-features --locked` for core proof.
The checksum tests cover exact/hash sensitivity, disabled/unrelated/regex rules,
missing/malformed reports, metadata and dual JSON/list preservation, count/byte
paging, exclusion of unconfirmed proposals and stale/deleted/wrong-node rules.

Use `python scripts/verify-native.py run --feature checksums` for owned native
window/input proof. The Python fixture adds an enabled checksum rule and reports
different hashes. Tab/Space opens the separate review, OCR requires Saved/Reported
values, and input returns to answer the permission. Require an actual once reply,
persisted decision, zero rule commands and every cleanup boolean. This proves
opening/reading and returning between windows; it does not prove native update
buttons. Those have Kit input, gRPC ACK and persistence proof.

## Bounds and limitations

- One review window and one background read/prepare worker. Pages contain at
  most 16 rules / 512 KiB including copied differences. Lists are virtualized.
  The independent read-only connection uses a 512 KiB cache and 250 ms busy timeout.
- Only confirmed, enabled flat AND rules with a simple explicit executable path
  and simple MD5/SHA1 operands are compared. Regex, list-file and unreported hashes
  are preserved. Missing or malformed reports do not fabricate mismatches.
- A cache reread before each command rejects local edits/removal. The daemon
  protocol has no compare-and-swap; concurrent external edits remain last-writer.
  All-page iteration follows the current name cursor, caps attempts at 10,000
  and does not promise a frozen snapshot of external daemon changes.
- A successful update trusts the reported binary; it does not allow/deny the
  pending connection. No command is successful until its matching OK and local
  persistence finish. An ERROR may have changed live policy and requires refresh.
- While running, native close minimizes the review and polling continues; Stop
  then Close finishes owned work. Automated native close/minimize behavior and
  native selected/all update controls remain unverified.
- No test changes the system daemon or firewall; synthetic replies do not prove
  kernel enforcement. Native proof records the actual backend/compositor.

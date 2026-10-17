# Raw file downloads

The optional OpenSnitch downloader action saves raw URL bytes to explicitly configured local files. It is independent of normalized blocklists and daemon Live tasks. Source configure UI is empty; Rustleblower provides a native singleton profile/review window as a native improvement.

Read [implementation/bounds/source contract](../../../../docs/raw-downloads-plan.md). Automatic downloads default off, imported source profiles stage off, manual runs require a saved revision and exact destination review. Worker-based profile I/O avoids filesystem work on the UI thread. Unknown source plugin metadata survives interchange without executing other actions.

## Native input and action proof

The journey stages imported settings OFF without writes/HTTP, reviews exact
URL/destinations, cancels review, refuses interval 0, saves 300 Minutes and reopens
without immediate scheduling. Actual reviewed runs publish exact binary bytes;
HTTP 500 and cancelled held GET preserve last-good content. Real AskRule/Deny
completes while HTTP is held. Actual backend mailbox success/error notices arrive;
cancel is quiet. A frozen RunNow revision refuses external settings changes
without HTTP or file mutation. Export preserves unknown plugin metadata and
refuses an existing file. Synthetic RawDownloads action opens/reuses its owned
dialog while retaining the hidden dashboard, following Updates semantics.

Run `python scripts/verify-headless.py --feature raw-downloads` for the input/action
checks above, using its owned loopback HTTP fixtures.

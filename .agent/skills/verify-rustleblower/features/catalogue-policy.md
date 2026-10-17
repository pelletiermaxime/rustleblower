# Temporary catalogue policy

Read [source contract, status and bounds](../../../../docs/catalogue-policy-plan.md). The native preference worker, inclusion/status model, backend lifecycle, schema11 observation markers, frozen query/export policy, Rules status and settings singleton are implemented. Source eight-preset ordering is exact; custom/always remain included. Derived cached expiry is separate from reported daemon enabled. A process-random marker distinguishes restored/current-run observations; pending and unavailable origin remain Unknown.

The searchable `rules-cache-node` control reaches up to32 cached nodes while offline. The `rules-catalogue-preferences` entry opens a singleton with `catalogue-ignore`, nonsearchable `catalogue-threshold`, `catalogue-review`, `catalogue-confirm`, `catalogue-keep-editing`, `catalogue-reload` and `catalogue-close`. Changing cache selection sends no daemon command. The exact reviewed ignored/kept list and captured preference revision govern Save.

## Simplified rule status display

Application rules shows the daemon-reported Enabled/Disabled state without
routine Active in UI cache, duplicate Disabled or Expiry unavailable labels.
Only estimated temporary-expiry warnings remain: Appears expired and
May have expired (previous UI session), in the warning color. Awaiting daemon
observation remains visible for unconfirmed lasting prompt decisions.
Underlying policy/status computation, wire values and enforcement are unchanged.

## Verification

Run `python scripts/verify-headless.py --feature catalogue-policy`. Review/save
temporary-rule filtering, preserve custom/always values, refuse stale settings,
answer an independent permission, then reopen the owned database in a fresh
process. Offline node selection and restored expiry warnings must preserve the
exact daemon-reported rule values and send no policy command.

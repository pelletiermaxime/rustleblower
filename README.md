# Rustleblower

A small native Rust + GPUI Kit frontend for the existing OpenSnitch daemon.
Rustleblower implements OpenSnitch's **UI-side gRPC server**; the daemon continues
to own interception, enforcement, and persistent rules.

The current version supports a local Unix socket, connection status, a separate
permission window with **Allow / Deny / Reject** and duration/match options, searchable disk-backed
connection history with on-demand details, and an **Application rules** page. You can search permanent
or temporary rules, change allow/deny/reject, enable/disable, delete, or create a
permanent rule for an executable. The full editor adds conditions, durations,
priority, logging, renaming and duplication. Changes are marked applied only
after the daemon confirms them. You can also import/export OpenSnitch JSON and
apply bulk actions to selected or filtered rules. Remote daemons/TLS, advanced
firewall controls, and a tray icon are still being ported.

## Install and run

Download a Linux x86-64 package from the
[GitHub releases](https://github.com/pelletiermaxime/rustleblower/releases).
This is an early development release with an unfinished interface. Packages are
built on Ubuntu 24.04 and require glibc 2.39 or newer, a Wayland or X11 desktop,
an installed font and a supported graphics driver. Install the OpenSnitch daemon
separately; Rustleblower contains only its frontend.

For the AppImage:

```sh
chmod +x rustleblower-0.1.1-linux-x86_64.AppImage
./rustleblower-0.1.1-linux-x86_64.AppImage
```

Without FUSE, add `--appimage-extract-and-run`. On Ubuntu/Debian, the `.deb`
installs library dependencies and adds the application to your launcher:

```sh
sudo apt install ./rustleblower_0.1.1_amd64.deb
rustleblower
```

The tar archive supports manual installation and includes runtime instructions.
`SHA256SUMS` verifies all three downloads. Run the app as your desktop user.
Building from source requires Rust 1.99, Python 3.12+, `patch` and the Linux
libraries listed in [the development guide](docs/development.md). Use
`python3 scripts/cargo.py run --locked`; the launcher prepares the small transport
patch on its first run.

By default, Rustleblower listens on `/tmp/osui.sock` and stores history under
`$XDG_DATA_HOME/rustleblower/history.sqlite3` (normally
`~/.local/share/rustleblower/history.sqlite3`). It refuses to replace any existing
socket or file. To switch from opensnitch-ui, quit that UI first and verify that
it has released the socket. The daemon can then reconnect to Rustleblower.
Rustleblower never edits the daemon configuration during subscription.

Closing either window minimizes it. **Quit Rustleblower** stops the server;
unanswered in-flight requests receive the configured one-time fallback where
the RPC is still alive. A disconnected daemon follows its own configured policy.
Prompts expire after 30 seconds by default, applying **deny once**. Launch defaults
come from `--prompt-timeout SECONDS` and `--timeout-action allow|deny|reject`.
Saved overrides in **Popup preferences… → Conditions & timeout** take precedence.
The fallback also applies when the pending-prompt limit is reached.

By default, the permission window starts with **Once** and an executable/destination match.
Choose **Rule options and details…** for a timed decision, Until restart, or Forever. Select
an executable, command, user, destination, domain, network, or other available
target, then add conditions with AND. Priority, logging, and reported checksums
are also available. Broader targets can affect other applications or destinations;
review them before answering. **Connection details** shows live arguments,
environment, checksums and process ancestry. Those live values are not saved in
history. Each request starts from your saved defaults; extra conditions do not
carry over from the previous request. Clicking or typing in the permission popup
pauses the normal countdown so you can review it. Opening advanced options by
default also pauses it. The popup then shows the remaining request limit: the
daemon's deadline still applies, with a maximum of two minutes from arrival.
Unanswered requests use the captured once-only fallback before that limit.
Each queued request keeps its own timer; interaction does not pause the others.

Choose **Popup preferences…** in the main window to save a default duration,
match target, priority and logging. An unavailable target falls back to an
executable and destination match. Defaults are captured when a request arrives, so changing them
does not alter requests already queued. Disabling **Show permission popups**
answers new requests immediately with the configured one-time fallback. Existing
requests keep their deadline. The main window displays a reminder while popups
are disabled. These UI preferences are saved beside the selected history database
as `history.sqlite3.preferences.json`; they do not change daemon configuration.

The **Conditions & timeout** tab can open rule options on each request and add
destination IP, destination port, user ID and a reported checksum to the default
match. Missing requested fields open the options with a warning for review.
Only values actually reported by the daemon become conditions.

Set a timeout from 1 to 110 seconds and choose Allow once, Deny once or Reject
once for the fallback. Leave the timeout blank and choose **Use launch setting**
to inherit the launch defaults. The daemon's own request deadline can shorten
the wait. Timeout, fallback and match settings are captured when the request
arrives; later preference changes do not alter queued requests. Automatic replies
always apply once, even when the selected rule duration is lasting.

**Popup preferences… → Placement** saves Center or one of the four corners for
the X11 primary display. Placement uses the usable workspace and includes native
window decorations. A queued request keeps the position captured when it arrived.
Expanding or manually moving a popup does not re-anchor it. On native Wayland,
the compositor chooses placement and the corner selector is disabled with an
explanation. Saved corner preferences remain available for later X11 sessions.

If a reported executable checksum differs from an enabled saved rule, the
permission popup shows **Review saved checksums…**. Its separate window lists
the affected rules and their saved/reported MD5 or SHA1 values. You can confirm
an update to the selected rule or update all changed rules across pages. Other
conditions, actions, durations and metadata stay intact. Updates count as complete
only after daemon confirmation; Stop prevents further unsent changes. A failure
shows how many updates were confirmed and requires a fresh inventory before retry.
Updating checksums does not answer the pending connection. Missing or malformed
reported hashes are skipped, and unreported hash conditions are preserved.
The comparison covers enabled flat AND rules with an explicit executable path
and exact checksum conditions; regex and list-file hash predicates are not rewritten.

A lasting popup reply appears in Application rules as **awaiting daemon
observation**. The protocol does not acknowledge installation of popup replies.
The entry becomes editable when the daemon reports its rule with a connection.
For rules that do not log or have already expired, use **Refresh connection** to
request a fresh inventory. Refresh is unavailable while a permission decision or
rule command is pending. It keeps the UI socket open while asking the daemon to
reconnect.

History shows reports received from the daemon and local prompt decisions.
A decision records the reply chosen by Rustleblower; it does not prove the daemon
received or enforced that reply. History timestamps are UTC. Use **Older** to
browse earlier pages and **Latest** to resume following new reports.

## Nodes and daemon configuration

Open **Nodes** or press Ctrl+4 to browse connected nodes and their last cached
metadata. Select a row, then **Use node for application rules** to choose that
node's catalogue. The header identifies the node selected for rules. Permission
popups identify the node that made each request and remain independent of the
selected page or node.

**Start/Stop interception…** asks for confirmation and waits for the selected
daemon's reply. **Daemon configuration…** opens settings grouped under General,
Logging, Firewall & memory, Statistics and Connection. These include the daemon's
default action/duration, process monitor, rule paths/checksums, logging, packet
queue and statistics limits, and connection/certificate options. Paths refer to
the daemon's machine. Changing its connection options can disconnect it from
this UI; Rustleblower currently serves the local Unix socket.

Choose **Review changes** to see the changed values, then **Confirm apply** to
send them to that daemon. Ctrl+Enter reviews; Ctrl+Shift+Enter confirms after
review. **Complete JSON** provides a multiline editor for the whole reported
document. Unedited, unrecognized and unreported settings are preserved; opening
the form does not populate missing defaults. The document must remain a JSON
object below 64 KiB. These settings change the daemon's saved configuration and
are separate from Rustleblower's local popup preferences.

A rejected or timed-out command leaves the last confirmed values visible with
an uncertainty warning. **Refresh node inventory** asks only that node to
reconnect, and is unavailable while it has a permission or command waiting.
Reconnecting one node does not cancel another node's prompts or rule changes.
Closing a daemon transport releases its connection slot.

Up to 32 distinct node names can be active over the local Unix socket. A new
subscription with the same name replaces that node's previous session. The
metadata cache retains at most 32 nodes; offline entries can be evicted as new
nodes arrive. Controls require the matching live session and notification stream.
TCP/TLS connections, address-based remote identity, live resource monitors and
cross-node bulk copying are still being ported.

## Statistics

Open **Statistics** for application, host, address, destination-port, user and
protocol counters. Search the reported values, choose a cached node, sort by
name or hits, or enable the top-ten chart. **Show matching history** opens the
retained connections for the selected value. Ctrl+1, Ctrl+2 and Ctrl+3 open
History, Application rules and Statistics; Ctrl+4 opens Nodes. Ctrl+F focuses
the current search.

These are the daemon's reported counters, saved locally alongside history.
They can count traffic that is absent from retained events. Omitted values keep
their last report until local eviction; a daemon restart starts a new counter
lifetime. The cache keeps up to 10,000 keys across 32 nodes, with a conservative
2 MiB counter-data budget and the configured retention age. It is a bounded
cache, so totals are not a permanent audit record.

**Next page** and **Previous page** browse a fixed view while new reports arrive;
**Latest** captures the current cache and resumes following reports. **Export
counters…** exports every match in that captured view to CSV or JSON Lines, or
copies the current page. Existing files require explicit replacement.

**Reset selected…** and **Reset category…** set a local display baseline after
confirmation. Later reports show increments beyond that baseline. They do not
reset the daemon or change firewall policy. A selected reset can also remove
matching retained history if you explicitly check that option. Category reset
covers the whole category for the chosen node scope, including values hidden by
search. A decreased reported count starts that key's baseline over.

## Filter, arrange and export history

Choose **All**, **Allowed**, **Denied** or **Rejected** for the quick action
filter. **Filters, columns & export…** opens the detailed controls:

- **Filters** combines up to eight conditions with AND: node, addresses/ports,
  executable/command, UID/PID, matched rule/duration, time, logging, priority and
  event reason. Use Equals, Contains, numeric/time comparisons or Not reported /
  Reported. Time values accept RFC3339 UTC; booleans use 0 or 1. Choose **Apply
  filters** to update the main view alongside its quick search/action filter.
- **Columns** switches between the compact summary and a virtualized table.
  Choose the columns and **Save history view**. The choice survives restart.
  Table header arrows sort all matching history; Older keeps a fixed arrival
  boundary while paging. Column widths can be resized for the current view.
- **Export** saves all matching history as CSV or JSON Lines using the chosen
  columns. **Copy matching page as CSV** copies at most 200 rows. Search, action,
  sort and page position are captured when this window opens; the conditions
  shown in its Filters tab also apply. Reopen the window to capture main-view
  changes. File export ignores the page position and includes all matches in its
  snapshot. JSON Lines contains the selected displayed values as strings.

Export runs in the background and leaves an existing destination alone unless
**Replace an existing export file** is selected. **Stop export** leaves the
previous destination unchanged. Files are capped at 256 MiB and clipboard output
at 4 MiB; narrow the filters for larger histories. The private export snapshot can
use additional temporary disk space. App database and settings paths cannot be
used as export destinations.

Older Rustleblower histories gain searchable metadata in background batches.
An indexing message explains when older fields are still incomplete. Missing
historical values stay blank; they are not treated as UID/PID zero. Metadata
sorting waits for indexing to finish. Environments, process checksums and live
process trees remain outside stored connection history.

## Test fresh traffic with the real daemon

After switching UIs, Rustleblower's header should show the daemon name/version.
Its history database is separate from opensnitch-ui's; existing UI history is
not imported. In Rustleblower choose **All** and clear the search, then run:

```sh
curl -I --max-time 60 https://example.com
```

Look for a new curl entry. Existing rules can allow or deny the request without
showing a permission prompt; rules with logging disabled may suppress its event.
A dashboard stuck waiting for the daemon is a connection problem, not evidence
that no traffic occurred. After rebuilding, use **Quit Rustleblower** before
relaunching; closing the window only minimizes the old process.

## Try it without changing the daemon

In one terminal:

```sh
mkdir -p .verification/demo
python scripts/cargo.py run --locked -- --socket "$PWD/.verification/demo/ui.sock" \
  --database "$PWD/.verification/demo/history.sqlite3"
```

In another, with OpenSnitch's Python package installed:

```sh
python scripts/fake-daemon.py --socket "$PWD/.verification/demo/ui.sock"
```

The fixture reports synthetic events and asks two permission questions. It makes
no network connections to the displayed destinations and changes no firewall rules.
It also supplies synthetic application rules and acknowledges edits to its own
in-memory catalogue. Add `--reject-rule-changes` to exercise an error response.

## Allow an application after a missed prompt

Open **Application rules**, search for the executable or rule name, and select
the matching rule. Choose **Allow** to change its action while preserving its
conditions, duration, priority, and logging settings. To add a rule for an
application that has none, choose **New application rule**, enter its absolute
executable path, and choose **Allow always**. A narrower existing deny rule can
still take precedence according to the daemon's rule ordering; review matching
rules when more than one applies.

The page waits for the daemon's confirmation. A failed command keeps the last
confirmed entry visible. The daemon can change memory before reporting a disk
error, so the page warns that live rules may differ until you reconnect and
reload its inventory using **Refresh connection**. If confirmation times out or the daemon disconnects,
reconnect before retrying. Delete asks for confirmation.

Choose **Edit…** for the name, description, action, duration, enabled state,
priority and logging options. Match conditions cover executable/parent paths,
commands, users, destinations, source addresses, ports, protocols, interfaces,
environment variables, checksums, CIDR networks and daemon list directories.
Multiple conditions form an AND group. **Match JSON** exposes the OpenSnitch
operator representation. **Duplicate…** opens a draft with a new name and keeps
the original rule. **All rule options…** opens the editor from the new-rule form.

Renaming saves the new rule before deleting the original and reports either
failure explicitly. Custom durations use daemon units, for example `90s` or
`2h30m`. Checksums and watched lists require the corresponding daemon settings
and files; paths are interpreted on the daemon's machine.

## Inspect a connection or create its rule

Select a history row and choose **Details…**. The Overview shows the recorded
node, executable, working directory, UID/PID, addresses, ports, protocol and
action. Arguments have their own tab and retain their individual boundaries.
Use **Copy details** or **Copy command line** to copy the recorded values.

The **Matched rule** tab shows the rule reported with that event. **Open current
rule…** loads its current entry for editing; it can differ from the historical
snapshot or may have expired. **Application rule…** starts a permanent rule for
the executable. **Application + destination…** also matches the observed host/IP,
destination port and protocol. Both open the full editor for review and wait for
daemon confirmation when saved. Existing deny/priority rules can still affect
the result according to the daemon's ordering.

Older history entries keep their summaries; detailed metadata is available for
new reports. Shortened fields are labelled, and incomplete executable/destination
values cannot automatically become match conditions. Creating or renaming a rule
checks the current catalogue for an existing name; use Edit on an existing rule
to change it explicitly.

## Import, export and bulk changes

Use **Import…** to read a JSON file, a directory containing OpenSnitch JSON rule
files, or JSON from the clipboard. Review the rules, then choose **Apply reviewed
rules**. Existing names are skipped unless **Replace rules that have the same
names** is checked. Single objects, JSON arrays, and OpenSnitch's adjacent-object
clipboard format are supported.

**Export filter…** includes every matching rule across all pages. **Export
selected…** includes checked rules, or the highlighted rule when none are checked.
You can copy JSON, save a JSON array file, or create a new directory containing
individual OpenSnitch rule files. Directory exports never merge into an existing
export. File exports replace their destination only after the complete snapshot
has been written successfully.

Check individual rules or use **Select page**, then choose a bulk action. Checked
names stay selected while paging; changing the text/duration filter clears them.
The review window can expand the operation to **every rule matching the current
filter, across all pages**. Bulk actions include allow/deny/reject, enable/disable,
and deletion. Each rule waits for its daemon reply. **Stop after current rule**
leaves confirmed changes applied and retains remaining rules for review or retry.
An error stops the batch and reports its confirmed and remaining counts.

## Resource limits

- At most 32 pending prompts, each limited to 64 KiB encoded and 256 KiB of
  retained connection allocations. At most 128 unobserved lasting popup replies
  are stored in the rule catalogue, sharing its rule count and byte limits.
- History ingestion is limited to 32 queued commands and an 8 MiB record budget,
  shared with reports received while the daemon is connecting.
  Each event can retain a detail snapshot of at most 24 KiB. Arguments are limited
  to 64 entries / 8 KiB total / 1 KiB per argument. Environment variables,
  checksums, and process trees are not saved in history.
- The UI holds one history page of at most 200 rows shared by its compact and table views. Both render only visible rows.
- Application rules are stored on disk and queried in pages of 200. Inventories
  are limited to 10,000 rules and 2 MiB of encoded data; an individual rule is
  limited to 64 KiB. At most 16 rule commands can await daemon replies.
- Checksum review holds at most 16 affected rules / 512 KiB per page. One
  background reader serves checks and update preparation, with a 512 KiB SQLite
  cache, no memory mapping and a 250 ms busy timeout. One checksum review window
  can be open; updates send one command at a time and recheck the saved rule first.
- SQLite uses one writer thread, a 2 MiB suggested page cache, disabled database
  memory mapping, and short write batches. Rule exports and import reviews use one
  independent read-only snapshot with another 2 MiB suggested cache. RPCs never
  wait for a database write.
- A rule import/review is limited to 10,000 rules, 2 MiB of encoded rule data,
  and 8 MiB of retained rule allocations. JSON input/output is capped at 8 MiB,
  with a 512 KiB cap per imported JSON object. JSON arrays are read one object at
  a time. Bulk changes send one command at a time; lists remain virtualized.
- Default retention is seven days from receipt, with a 256 MiB disk target.
  `--history-days` and `--history-mib` change these. A conservative logical-data
  budget reserves space for indexes and WAL; the main database also has a page
  limit. SQLite checkpoints its WAL after each write batch. The disk target is
  not a filesystem quota, especially if another process opens the database.
- History warnings report the cause of missed events during the current run.
  Normal connection startup preserves early reports without a warning.
  Ping acknowledgements
  confirm RPC receipt, not durable history storage. Queue saturation can lose
  telemetry; it cannot consume the prompt queue.

Compatibility currently targets OpenSnitch 1.7.2. Oversized gRPC messages are
rejected at 2 MiB; unusually large rule inventories may exceed that limit.

Run `python scripts/verify-headless.py` for background GPU/input verification.
For an owned foreground session, build first, then run
`python scripts/verify-native.py doctor` and
`python scripts/verify-native.py run --feature history` (or `--feature prompts`).
These reuse FerryDock's verification patterns. The generated
[verification skill](.agent/skills/verify-rustleblower/SKILL.md) documents launch,
doctor, drive, evidence, cleanup, and the feature map.

Developer commands and architecture are in [docs/development.md](docs/development.md).

## License

MIT. Rustleblower maintains its own wire-compatible protocol schema and generated
bindings. The OpenSnitch daemon is a separate GPL-licensed project and is not
bundled. Adapted FerryDock helpers and dependencies retain their own notices.

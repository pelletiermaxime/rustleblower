# User guide

For installation and first launch, see the [README](../README.md).

- [Settings and desktop behavior](#settings-and-desktop-behavior)
- [Permission decisions](#permission-decisions)
- [Nodes and daemon configuration](#nodes-and-daemon-configuration)
- [Alerts](#alerts)
- [System firewall configuration](#system-firewall-configuration)
- [Live sockets and process details](#live-sockets-and-process-details)
- [Statistics](#statistics)
- [History filters and exports](#filter-arrange-and-export-history)
- [Connection troubleshooting](#check-that-the-daemon-is-connected)
- [Missed prompts and application rules](#allow-an-application-after-a-missed-prompt)
- [Blocklist subscriptions](#blocklist-subscriptions)
- [Connection details and rule creation](#inspect-a-connection-or-create-its-rule)
- [Rule imports, exports and bulk changes](#import-export-and-bulk-changes)

## Settings and desktop behavior

Open **Settings** in the top-right corner for one large window with categories
on the left: appearance and language, popup preferences, startup, updates, history,
rule catalogue, reputation, raw downloads and table highlighting. Switching
categories keeps your unsaved edits. Every page has **Close** at the bottom right,
with **Save** alongside it on editable pages. Save commits the current page;
changes that need review show that review before the final Save.

Use **Settings → Appearance** to choose **System**, **Light** or **Dark**, then
click **Save**. The saved choice applies to every open window and is stored beside
this history database. System follows the desktop's color appearance. Open pages, rule drafts and pending permission
requests stay open when switching colors. Closing settings discards unsaved
appearance choices. If saving fails, the applied appearance stays unchanged and
the draft is retained for correction or retry. **Use System appearance** stages
the default; click Save to apply it. **Display density** and installed native palettes are also available. Density changes text, spacing and ordinary controls;
fixed window sizes and table row heights stay the same. Density and colors have
separate reset controls. A missing saved palette falls back to its base color
mode with an explanation. **Language** offers the system language and installed
catalogues; a saved choice takes effect on the next launch. Packages bundle English, French, German, Spanish and Japanese, each covering the full
message catalogue. Missing messages in externally installed partial catalogues
fall back to English.

Use **Updates…** to check official Rustleblower releases or the selected daemon's
reported version. Automatic checks start off. **Check now** uses the saved choices;
you can also enable checks on startup or at a chosen interval and include
prereleases. An available-update notification opens the same preferences window.
The result links to the official release page.

**Reputation…** provides optional VirusTotal checks for domains, IP addresses and
reported executable hashes. Checks start off. Enabling them sends the selected
values to VirusTotal using your API key; choose a private key file in preferences.
You can exclude destinations, choose a warning threshold and limit requests per
session. Permission prompts remain answerable while a report loads. Reports
distinguish warnings, unflagged analysis and unavailable information; a warning
may recommend **Reject once** while the prompt is untouched. Your permission
decision and its captured timeout fallback remain independent of the check.

**Raw downloads…** manages scheduled URL-to-file downloads separately from
blocklist subscriptions. Import an OpenSnitch downloader profile, review its exact
URLs and destination filenames, then save. Automatic downloads start off and wait
the chosen interval before their first run. You can run a saved group manually or
cancel it; failed or cancelled downloads preserve the previous file. Success and
failure notifications can open this window. Downloads do not create firewall
rules or activate downloaded lists.

The experimental **Data usage** page (`Ctrl+8`) shows
per-application upload/download rates and totals since monitoring started.
It uses a separate Linux collector while the desktop app stays unprivileged.
Stale or unavailable collection is shown explicitly. Start monitoring opens the
system authentication dialog for the collector; see
[Data usage setup and limitations](data-usage.md).

The main window remembers its size, maximized/fullscreen state and last page
for the next launch. Its saved bounds are fitted to connected displays when it
reopens; on Wayland, the compositor controls its position. Changes are saved in
the background after resizing settles, and on close or Quit. Loading or saving
this layout does not hold permission requests. Splitter positions and column/filter
restoration outside History are separate features still being ported.

History remembers its search text, action filter, applied advanced conditions,
table sort and column widths alongside the selected columns and refresh interval.
It reopens on the first matching page so an old page cursor cannot hide new
traffic. These choices save automatically in the background. If the settings
file cannot be read or changes externally, Rustleblower preserves it, uses
defaults or session-only changes and shows an explanation in History.

To launch Rustleblower when you log in, open **Settings → Startup** in the main window or
**Startup preferences…** in the tray menu. Select **Start Rustleblower when I log
in**, then **Save startup preference**. The entry uses this installed binary or
AppImage and the current socket, history location and permission fallback.
Keep an enabled AppImage at that location; save the preference again after
moving it. Uncheck and save to disable automatic startup. Closing without saving
keeps the previous setting; **Reload** discards the staged selection. This
controls Rustleblower's own entry and preserves OpenSnitch's startup files.

Closing the main window hides it to the system tray beside the clock. Click its
icon or choose **Open Rustleblower** from its menu to reopen it. Permission
popups still appear while the main window is hidden. If the desktop has no
compatible system tray, closing the main window minimizes it instead. Closing
a permission popup also minimizes it. **Quit Rustleblower**, in the dashboard
or tray menu, stops the server;
unanswered in-flight requests receive the configured one-time fallback where
the RPC is still alive. A disconnected daemon follows its own configured policy.
Prompts expire after 30 seconds by default, applying **deny once**. Launch defaults
come from `--prompt-timeout SECONDS` and `--timeout-action allow|deny|reject`.
Saved overrides in **Popup preferences… → Conditions & timeout** take precedence.
The fallback also applies when the pending-prompt limit is reached.

## Permission decisions

The permission window asks whether an app may access the network, with
**Allow forever** and **Allow for 12 hours** directly available. Choose between
**any destination** and **only the shown destination** using the two radio choices.
A new installation defaults to the whole app; existing saved scopes are preserved.
The destination-only choice matches the exact reported hostname, or explicitly
shows an IP address when no hostname is available. It covers any port and never
silently creates a wildcard domain rule. The sentence above the buttons previews
the selected scope. Block keeps the selected duration and displays it on its button.

The app's icon appears when it can be matched to an installed desktop app.
Otherwise, the window shows initials beside the app name. Its executable path
stays visible, and you can answer immediately while the icon loads.

**Details and advanced rules…** contains other durations, command/user/domain/network
scopes, additional conditions, priority and logging. All additional conditions must
match; fields already required by the scope are omitted from the checkboxes.
Saved custom conditions are shown as a Custom rule until explicitly changed.
Choosing either simple scope replaces those conditions for this request, including
any old address or port restriction. It does not overwrite saved preferences.
Both simple choices are unavailable when the executable was not reported.
**Connection details** shows live arguments,
environment, checksums and process ancestry. Those live values are not saved in
history. Each request starts from your saved defaults; extra conditions do not
carry over from the previous request. Clicking or typing in the permission popup
pauses the normal countdown so you can review it. Opening advanced options by
default also pauses it. The popup then shows the remaining request limit: the
daemon's deadline still applies, with a maximum of two minutes from arrival.
Unanswered requests use the captured once-only fallback before that limit.
Each queued request keeps its own timer; interaction does not pause the others.

Choose **Popup preferences…** in the main window to save a default duration,
rule scope, priority and logging. An unavailable target falls back to an
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

While Rustleblower is connected, the daemon denies unmatched traffic while a
permission request is unanswered, even if its saved default action is Allow.
This matches opensnitch-ui and does not change the saved daemon configuration or
existing application rules. The saved default applies when the UI is disconnected;
the popup timeout action is a separate preference.

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
this UI; review the [connection options](remote-transport.md) before applying them.

For a selected node, **Remember this node's paused state after reconnect or app
restart** is off by default. Enable it to keep that node's last confirmed state.
After a confirmed Stop, Rustleblower requests a pause if the same node reconnects
running. It waits for the daemon's reply; permission popups remain usable. It
never starts a stopped daemon automatically. Turning the option off leaves the
current interception state alone. Each node has its own choice; the setting is
local to Rustleblower and follows the bounded node cache.

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
For TCP/TLS connections and remote node identity, see [connection options](remote-transport.md).
Use **Live** for reported resources and **Copy to nodes…** for reviewed rule copies.

## Alerts

Open **Alerts** or Ctrl+5 for daemon messages and typed process, connection, rule
or firewall alerts. Search their text and payloads, enter an exact node name,
filter severity or unread status, and select a row for details. **Copy complete
JSON** includes the supplied payload; the preview explicitly labels shortening.
Typed alerts can include the process metadata the daemon sent.

**Older/Previous** browse a captured view; **Latest** follows new arrivals.
Marking read or confirming deletion changes only the local archive. **Delete
matching…** covers all current filter matches through the captured arrival
boundary, preserving new alerts that arrive during confirmation.

Enter an absolute path to export all matches to CSV or JSON Lines. Existing files
require **Replace**; **Stop** cancels before the completed export is published.
The archive retains at most 10,000 alerts / 2 MiB of logical data and uses the
configured history age. Oldest alerts may be evicted. Skipped alerts have their
own warning and do not inflate connection-history loss. Per-rule alert settings
are still being ported.

Daemon alerts can also appear as desktop notifications. Each notice identifies
its node and urgency; **Open alerts** restores the Alerts page even when the
main window is closed to the tray. Turn these off in **Popup preferences** while
continuing to retain alerts and receive permission popups. Desktop notification
visibility follows your system settings, including Do Not Disturb. Bursts replace
the current desktop notice; individual retained records remain in Alerts.

## System firewall configuration

Select a node in **Nodes**, then open **System firewall** to browse its reported
chains and rules. Search within a chain and page through its rules. **Complete
JSON** lets you review and apply a complete configuration while retaining rule
order and expressions. The daemon must acknowledge a reload before the local
snapshot changes; the window distinguishes its reported configuration from an
accepted reload awaiting a fresh report. An acknowledgement alone does not prove
that every rule is enforced.

Use **New/Edit chain** and **New/Edit rule** for field-based editing. Rule forms
include ordered expressions and key/value pairs, duplication, enabled state and
legacy fields. Move controls change array order while preserving the separate
insertion-position metadata. New rules default to position zero and need an
expression before they can be enabled. **Keep in draft** stages each edit; the daemon changes only after
the complete draft is reviewed and applied. An open form can still keep its local
values after a reconnect, so you can copy them before reopening the current node.

Use **Add guided statement…** for ports, addresses,
interfaces, connection state/marks, ICMP, logging, quotas, counters and rate
limits. **Choose guided verdict…** helps enter a target and its parameters for
the current chain. Helpers stage a draft and validate their inputs; the raw
editor remains available for advanced or unfamiliar expressions.

**Alt+E** edits the selected chain. **Ctrl+Enter** keeps the current form step or
reviews the complete draft; **Ctrl+Shift+Enter** applies a prepared review.

**Files and profiles** imports a full configuration into the local draft, merges
OpenSnitch `.profile` files into matching chains, and exports complete configurations
or the selected chain. It also lists profiles from a chosen folder (or OpenSnitch's
standard profile folder). Export creates a new file and preserves existing files.
Removing a profile's rule IDs preserves the chain policy; review that policy before
applying.

**Policies and state** provides built-in Accept/Drop presets for supported
input, output and forward chains. **Enable system firewall** stages a state
change. Disabling prepares accept policies while active before saving Disabled;
the review shows each step, and the app waits for its confirmation before moving
on. Closing this editor lets the reviewed operation continue in the background.
**Alt+P** opens this page; **Ctrl+Shift+E** toggles its draft state.
Editing requires a reported nftables backend and supported format; offline or
unsupported configurations stay read only. Application rules and
permission prompts continue through their separate workflows.

## Live sockets and process details

Open **Live** (**Ctrl+6**) and start monitoring the node selected through **Nodes**.
Choose a refresh interval and, for sockets, protocol, address family and state.
**Ctrl+Enter** starts and **Ctrl+Shift+Enter** pauses. Pause freezes the current
snapshot for searching, paging and copying. Selecting a socket with a known PID
lets you inspect that process; you can also enter a PID in **Process details**.
You can also select a connection in **History**, open **Details**, then choose
**Inspect process**. This uses that connection's daemon even if another node is
selected. It reads the PID now: the process may have exited or its PID may have
been reused. The original event and executable remain visible for comparison.

Process fields include status, open descriptors, I/O counters, memory maps, stack,
environment, arguments, parent tree and reported checksums. **Node resources**
shows uptime, load, process count and memory/swap from the daemon. You can copy
individual socket JSON, complete process fields or the entire admitted snapshot.

Leaving Live or closing its dashboard requests task stop. A disconnected node's
last data stays visible as stale and monitoring must be restarted explicitly.
OpenSnitch 1.7.2 does not acknowledge successful task stops, so the app reports
**Stop requested**. Live snapshots are bounded and volatile; connection history
continues to be stored separately on disk. Large snapshots show explicit
shortening feedback. Live inspection does not change firewall policy.

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

- **Filters** combines up to eight conditions in All/Any groups: node, addresses/ports,
  executable/command, UID/PID, matched rule/duration, time, logging, priority and
  event reason. Use Equals, Contains, numeric/time comparisons or Not reported /
  Reported. Time values accept RFC3339 UTC; booleans use 0 or 1. The optional
  **Formula** field combines the numbered conditions, for example
  `1 AND (2 OR 3)`. Reference each condition once; clear the formula to use group
  buttons again. Adding/removing a condition clears it because numbers change.
  Choose **Apply filters** to update the main view alongside its quick
  search/action filter. Application-rule conditions offer the same formula.
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

History → **History preferences…** controls retained days, the database size target
and cleanup while traffic is quiet. Review the settings and expired-record counts
before applying; Back leaves data unchanged. Smaller limits can remove older
history, daemon alerts and cached counters. Cached application rules and node
settings remain intact. Existing files may take time to compact, so the target is
not a filesystem quota.

**Display refresh** can update every 1–60 seconds or run manually with 0. Manual
mode keeps recording in the background; use **Refresh now**, search, paging or
Latest to load data explicitly. These preferences survive restart. Explicit
`--history-days` and `--history-mib` flags override and save retention values;
otherwise saved UI choices take precedence over launch defaults. The database
path is shown and can be copied from the preference window.

## Check that the daemon is connected

After switching UIs, Rustleblower's header should show the daemon name/version.
Its history database is separate from opensnitch-ui's; existing UI history is
not imported. In Rustleblower choose **All** and clear the search, then run:

```sh
curl -I --max-time 60 https://example.com
```

Look for a new curl entry. Existing rules can allow or deny the request without
showing a permission prompt; rules with logging disabled may suppress its event.
A dashboard stuck waiting for the daemon is a connection problem, not evidence
that no traffic occurred. After updating, use **Quit Rustleblower** before
relaunching; closing the window keeps the old process running in the tray (or
minimized when a tray is unavailable).

## Allow an application after a missed prompt

If a permission popup times out, Rustleblower can show a desktop notification
with the applied action. **Open application rules** takes you to the rules page.
Use the **Missed popups** tab in popup preferences to turn these notices off,
customize their text or restore the default. The template preview shows the
expanded message; `\n` inserts a line break, `\t` a tab and `\\` a backslash.
The general desktop-notification preference also applies.

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

Port rules can use inclusive source/destination ranges such as `4000-4010` through
the application rule editor's match-type picker. Rustleblower checks daemon
compatibility before applying a range rule; unconfirmed older builds show an
explanation and keep the draft.

## Blocklist subscriptions

Open **Blocklists** (Ctrl+7) and choose **Add subscription…** (Ctrl+N). Enter a
name and an HTTP(S) URL, choose the text format and save (Ctrl+S). Interval,
timeout and size each have a count and a unit. An empty count inherits the saved
default count; **Inherit default** in the unit picker inherits its unit separately.
The form shows the effective limits before saving. **Defaults…** changes the
global count/unit pairs, and dependent subscriptions follow those choices.
Configuration is created by the app;
no configuration file needs to be prepared manually.

Before saving, **Test URL** checks whether the address is reachable. For hosts
format it also inspects a short sample; the result is indicative and a very short
sample is marked inconclusive. The test follows redirects, can be cancelled, and
does not save or activate the list. Editing the URL or format discards the previous
result. **Save** remains a separate action.

Default changes are checked against every saved subscription before they are
committed. If a dependent limit becomes invalid, the previous settings remain
active and the editor explains the error. Reopen a subscription editor if defaults
changed while it was open. Older numeric limits retain their absolute meaning;
reimport an original OpenSnitch file to recover inheritance that an earlier
Rustleblower import had already converted into fixed values.

Automatic updates initially remain paused. **Refresh selected** checks one list
now; **Force refresh** skips its HTTP cache validators. **Start automatic updates**
enables the saved schedule when a local daemon is connected. You can pause or
cancel a running download without closing the app. Previous valid data remains
available after failed, invalid or oversized downloads. The page shows status,
errors and a bounded preview/history, and searches and sorts across all matching
subscriptions. Removing a subscription stops its updates and keeps downloaded
data; **Show removed** and **Edit / restore…** can restore it.

After refreshing a list, select it and choose **Review new list rule…**. The
application-rule editor opens a permanent Deny draft combining your user ID and
its watched directory;
check the displayed target node, action and conditions before **Save rule**.
Nothing is sent if you cancel. The change is saved only after the daemon confirms
it. **Inspect associated rules…** lists confirmed rules for that directory on the
selected node, including rules with additional conditions; select one to review
or edit it. Choose a different local target from **Nodes**.

Downloaded data is converted to the format expected by the daemon. Domain names
become hosts entries; hosts aliases are expanded, and IP lists can contain both
addresses and CIDR prefixes. Changing list type publishes into its own directory
and retains the old type's data for existing rules. Both count toward the disk
budget. A downloaded list alone does not block traffic: the daemon needs a rule
that references its directory and access to those files. Subscription configuration
and files are stored in a `subscriptions` folder beside the history database.
Choose **Rule collections…** (Ctrl+Shift+L while on Blocklists) to browse groups
and **All enabled subscriptions**, separated by list type. Search and the Group,
Type and Active sources sorts cover every matching collection. Each entry shows
its active source count and offers rule review and associated-rule inspection.
An empty installation already has five all-list collections; they match no data
until subscriptions are added and refreshed.

Changing a saved subscription’s enabled state, groups or input type, or restoring
it, opens a membership review before saving. **Back to form** retains the draft;
Cancel leaves the saved membership intact. Existing collection rules follow
confirmed membership changes and refreshed data. Pausing automatic updates keeps
using the last valid data. Retained direct-list rules continue to use their source
files independently of group/all membership.

For several lists, mark rows across pages and choose **Bulk edit…** (Ctrl+Shift+B).
You can also target every active subscription matching the current filter.
Enable or disable lists, add/remove/replace/clear groups, change input format, or
change interval, timeout and size overrides. Check a limit to change it, then
choose its count and unit independently. An empty count inherits the default
count; **Inherit default** in the unit selector inherits its unit. Unchecked
limits keep their existing overrides. **Review changes** (Ctrl+Enter) lists raw
before/after choices, the reviewed defaults and effective values across all
selected pages.
Back keeps the fields; Cancel leaves saved values intact. **Apply reviewed
changes** (Ctrl+Shift+Enter) saves the whole reviewed batch together. If any
reviewed list or the global defaults changed meanwhile, rebuild the review
before applying it.
Use **Import / export…** (Ctrl+Shift+T) to transfer subscription configuration.
Import accepts a Rustleblower configuration, an OpenSnitch subscription action
file, or its configuration object. Choose the JSON file and **Review import…**
(Ctrl+Enter). The review shows new lists, matching identities, replacements and restored
entries, with normalized groups and limits. Display names can be shared by
several subscriptions; imported extra fields remain attached to their internal
identities through renaming and restoration. Replacement and importing
global defaults are separate choices; current defaults and automatic-update
state are kept unless you select that option. **Replace complete active
catalogue** also reviews entries absent from the file: they stop updating and
leave collections, while their downloaded files remain available to existing
rules. Matching entries are replaced in that mode. **Import reviewed configuration**
(Ctrl+Shift+Enter) applies the complete reviewed change. Back preserves options;
Cancel changes nothing. Rebuild the review if the catalogue changed meanwhile.

Export can include marked rows or all active matches across pages, in native
Rustleblower or OpenSnitch action format. Choose a new destination filename;
existing files are preserved. Configuration transfer keeps downloaded data and
daemon rules separate. Imported action metadata is retained for export; other
plugin actions are retained without execution. Rustleblower uses
imported subscription notification settings and provide a **Notifications…**
editor for success/failure messages. Click a subscription notice to reopen
Blocklists; the general Desktop notifications switch also applies. Cancelled
downloads and URL tests stay quiet. OpenSnitch exports normalize typed filenames and make duplicates distinct;
native configuration preserves exact names and filenames. When upgrading older
imported metadata, uncertain matches stay archived rather than being attached to
an arbitrary row. Reimporting the original file can bind those fields again.
Archive inspection/cleanup and some subscription options remain under development.

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

In the rule editor, **Apply to nodes…** reviews the current unsaved draft for
selected connected nodes, including the current one. Cancelling that window
keeps the editor fields. Editing the same name enables replacement by default;
new-name conflicts require explicit replacement. For a rename, each destination
first confirms the new rule, then removes its old name if present. Stop waits
for the current reply; continuing skips confirmed work. If removal fails, the
new rule can already be saved while the old name remains: reconnect and rebuild
the review to finish only the outstanding work. Results show both counts.

**Import to nodes…** reads a file, a directory of rules, or an explicitly captured
clipboard into the same destination review. It applies exactly those reviewed
values, even if the original input changes later. Invalid captured clipboard
data reports an error; **Use file path** explicitly selects the file instead.
The ordinary **Import…** below targets the current node.


Use **Import…** to read a JSON file, a directory containing OpenSnitch JSON rule
files, or JSON from the clipboard. Review the rules, then choose **Apply reviewed
rules**. Existing names are skipped unless **Replace rules that have the same
names** is checked. Single objects, JSON arrays, and OpenSnitch's adjacent-object
clipboard format are supported.

**Export nodes…** saves rules from several cached nodes, including offline ones.
Choose nodes and a parent directory, then **Review export**. All rules are
included by default; the captured search/duration filter can be applied to every
selected node. The review freezes the contents and shows rule and unconfirmed
proposal counts. **Export reviewed folders** creates a new folder with one
subfolder per node and a manifest identifying them. To restore rules, import a
node subfolder. Existing exports are preserved; cancelling or an error before
completion leaves no partial export folder.

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

To copy existing rules to other connected nodes, use **Application rules → Copy
to nodes…**. Marked rules across pages are used first; the scope checkbox can
include every rule matching the current filter. Choose destinations, review the
counts and source rules, then apply. Existing names skip by default; replacing
them is explicit. Identical rules and names awaiting daemon observation skip.
Paths, user IDs and match conditions are copied exactly, so check their meaning
on each destination. Each node has its own confirmed/failed result. Stop waits
for the current reply, and continuing does not repeat confirmed work. After a
reconnect or uncertain error, rebuild the review; completed copies remain in
place. **Copy results** keeps the complete per-node result text. The source
node's rules stay unchanged.

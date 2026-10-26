# Rustleblower

A native Linux interface for [OpenSnitch](https://github.com/evilsocket/opensnitch).
See which applications access the network, answer permission requests, and manage
firewall rules. The OpenSnitch daemon continues to enforce your decisions;
Rustleblower replaces its desktop interface.

## Features

- **Connection permissions:** allow, deny or reject access, temporarily or
  permanently, for an application or a specific destination.
- **History and statistics:** search connections, inspect their details, filter
  traffic and export results.
- **Application rules:** create and edit rules, import/export OpenSnitch JSON,
  and apply reviewed changes to multiple rules or nodes.
- **Blocklists:** download and update subscriptions, then attach them to rules.
- **Monitoring:** view daemon alerts, live sockets and process details.
- **Desktop integration:** system tray, optional login startup, light/dark themes,
  and English, French, German, Spanish and Japanese interfaces.

Experimental per-application **Data usage** monitoring is also available. See its
[setup and limitations](docs/data-usage.md). Some opensnitch-ui features remain
under development.

## Install

Download a Linux x86-64 package from
[GitHub releases](https://github.com/pelletiermaxime/rustleblower/releases).
Install the **OpenSnitch daemon separately** and run Rustleblower as your normal
desktop user.

Packages require glibc 2.39 or newer, a Wayland or X11 desktop, an installed font
and a supported graphics driver.

### AppImage

```sh
chmod +x rustleblower-0.4.1-linux-x86_64.AppImage
./rustleblower-0.4.1-linux-x86_64.AppImage
```

Without FUSE, add `--appimage-extract-and-run` to the launch command.

### Debian / Ubuntu

```sh
sudo apt install ./rustleblower_0.4.1_amd64.deb
rustleblower
```

The package installs its library dependencies and adds Rustleblower to your
application launcher. A tar archive is available for manual installation.
Use the release's `SHA256SUMS` to verify your download.

## Connect to OpenSnitch

1. Quit **opensnitch-ui** before starting Rustleblower. Only one interface can
   use the default `/tmp/osui.sock` socket at a time.
2. Start Rustleblower from your application launcher or package.
3. Check that its header shows your daemon's name and version.

Rustleblower refuses to replace an existing socket. If it cannot start, make
sure the previous interface has fully quit. Existing daemon rules remain in
place; history from opensnitch-ui is not imported.

For a remote daemon, see [TCP and TLS connection options](docs/remote-transport.md).

## Everyday use

When a permission window appears, check the application and destination. Choose
**any destination** or **only the shown destination**, then block the request,
allow it for 12 hours, or allow it forever. **Details and advanced rules…** offers
other durations, Reject and more specific conditions.

While Rustleblower is connected, unmatched traffic is denied while a permission
request is unanswered, even if the daemon's saved default is Allow. Unanswered
prompts default to **Deny once after 30 seconds**. You can change the timeout and
its action in **Settings → Popup preferences**. When Rustleblower disconnects,
the daemon follows its own saved policy.

Use **History** to find a connection and open **Details…**. Use **Application
rules** to allow an application after a missed prompt or edit an earlier decision.
If a rule change is uncertain, use **Refresh connection** before trying again.

In **Blocklists**, add a subscription and refresh it, then choose **Review new
list rule…** to use it for blocking. Downloading a list alone does not block
traffic. Automatic subscription updates start paused.

Closing the main window keeps Rustleblower in the system tray; permission
windows still appear. Use **Quit Rustleblower** in the tray menu to exit fully
or before relaunching after an update. Without a compatible tray, closing the
window minimizes it instead.

For detailed workflows, see the [user guide](docs/user-guide.md).

## Settings and local data

Open **Settings** to change appearance, language, popup behavior, startup and
history retention. Enable **Start Rustleblower when I log in** under **Startup**
if you want it to launch automatically.

Connection history is stored locally in
`$XDG_DATA_HOME/rustleblower/history.sqlite3`, normally
`~/.local/share/rustleblower/history.sqlite3`. The default retention is seven days,
with a 256 MiB size target. **History preferences…** lets you change these limits.

Automatic release checks, VirusTotal reputation checks and scheduled raw downloads
start off. VirusTotal checks send selected domains, IP addresses or executable
hashes to VirusTotal using your API key. See the [user guide](docs/user-guide.md)
for these optional features.

## More information

- [User guide](docs/user-guide.md)
- [Data usage setup and limitations](docs/data-usage.md)
- [Remote connections](docs/remote-transport.md)
- [Release notes](https://github.com/pelletiermaxime/rustleblower/releases)
- [Building and development](docs/development.md)

## License

[MIT](LICENSE). The OpenSnitch daemon is a separate GPL-licensed project and is
not bundled. Third-party notices are included with the packages.

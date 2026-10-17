#!/usr/bin/env python3
"""Native input adapter adapted from FerryDock (MIT; see LICENSES/FerryDock-MIT.txt)."""
import argparse
import fcntl
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import time
import uuid
from prepare_dependencies import DESTINATION, is_prepared

ROOT = Path(__file__).resolve().parents[1]


def desktop_environment(requested, display, environment):
    env = environment.copy()
    backend = requested
    if backend == 'auto':
        backend = 'wayland' if env.get('WAYLAND_DISPLAY') else 'x11'
    if backend == 'wayland':
        if not env.get('WAYLAND_DISPLAY') or not env.get('XDG_RUNTIME_DIR'):
            raise RuntimeError('Wayland requires WAYLAND_DISPLAY and XDG_RUNTIME_DIR from your desktop session')
        env.pop('DISPLAY', None)
    else:
        if not display:
            raise RuntimeError('X11 requires DISPLAY or --display')
        env.pop('WAYLAND_DISPLAY', None)
        env['DISPLAY'] = display
    return backend, env


class NativeDesktop:
    """Own native input resources and validate the target before each interaction."""

    WAYLAND_KEYS = {
        'ctrl+f': ['29:1', '33:1', '33:0', '29:0'],
        'ctrl+a': ['29:1', '30:1', '30:0', '29:0'],
        'alt+e': ['56:1', '18:1', '18:0', '56:0'],
                'alt+p': ['56:1', '25:1', '25:0', '56:0'],
                'ctrl+shift+e': ['29:1', '42:1', '18:1', '18:0', '42:0', '29:0'],
        'ctrl+3': ['29:1', '4:1', '4:0', '29:0'],
        'ctrl+Return': ['29:1', '28:1', '28:0', '29:0'],
        'ctrl+shift+Return': ['29:1', '42:1', '28:1', '28:0', '42:0', '29:0'],
        'ctrl+4': ['29:1', '5:1', '5:0', '29:0'],
        'ctrl+6': ['29:1', '7:1', '7:0', '29:0'],
        'ctrl+7': ['29:1', '8:1', '8:0', '29:0'],
        'ctrl+shift+t': ['29:1', '42:1', '20:1', '20:0', '42:0', '29:0'],
        'ctrl+shift+b': ['29:1', '42:1', '48:1', '48:0', '42:0', '29:0'],
        'ctrl+shift+l': ['29:1', '42:1', '38:1', '38:0', '42:0', '29:0'],
        'ctrl+n': ['29:1', '49:1', '49:0', '29:0'],
        'ctrl+s': ['29:1', '31:1', '31:0', '29:0'],
        'ctrl+1': ['29:1', '2:1', '2:0', '29:0'],
        'ctrl+5': ['29:1', '6:1', '6:0', '29:0'],
        'alt+F4': ['56:1', '62:1', '62:0', '56:0'],
        'Escape': ['1:1', '1:0'],
        'Down': ['108:1', '108:0'],
        'End': ['107:1', '107:0'],
        'Tab': ['15:1', '15:0'],
        'space': ['57:1', '57:0'],
        'Return': ['28:1', '28:0'],
    }

    def __init__(self, backend, env, evidence, binary):
        self.backend, self.env, self.evidence, self.binary = backend, env, evidence, binary
        self.app = self.pid = self.window = None
        self.input_daemon = self.input_log = None
        self.input_socket = evidence / 'input.sock'

    def run(self, command, timeout=30, check=True):
        return subprocess.run(command, env=self.env, capture_output=True, text=True,
                              timeout=timeout, check=check)

    def doctor(self):
        tools = ['tesseract'] + (['kdotool', 'qdbus6', 'spectacle', 'ydotool', 'ydotoold']
                                if self.backend == 'wayland' else ['xdotool', 'magick'])
        missing = [tool for tool in tools if not shutil.which(tool)]
        if missing:
            raise RuntimeError('Missing native verification tools: ' + ', '.join(missing))
        if not self.binary.is_file():
            raise RuntimeError('Build the requested binary before benchmarking')
        if not is_prepared():
            raise RuntimeError('Transport dependency is not prepared; run python scripts/cargo.py build --locked first')
        # This module is cfg(test) and is not an input to the desktop binary.
        # Cargo correctly leaves that binary untouched after only its tests change.
        sources = ([path for path in (ROOT / 'src').rglob('*.rs') if path.name != 'kit_tests.rs' and 'kit_tests' not in path.parts]
                   + list(DESTINATION.rglob('*.rs'))
                   + list((ROOT / 'patches').glob('*.patch'))
                   + [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock', ROOT / 'scripts/prepare_dependencies.py'])
        if any(path.stat().st_mtime_ns > self.binary.stat().st_mtime_ns for path in sources):
            raise RuntimeError('Native binary is older than source; run python scripts/cargo.py build --locked first')
        if self.backend == 'wayland':
            socket = Path(self.env['WAYLAND_DISPLAY'])
            if not socket.is_absolute():
                socket = Path(self.env['XDG_RUNTIME_DIR']) / socket
            if not socket.is_socket():
                raise RuntimeError('Wayland display socket is unavailable: ' + str(socket))
            if not os.access('/dev/uinput', os.R_OK | os.W_OK):
                raise RuntimeError('Wayland input requires access to /dev/uinput; system permissions are not changed by this helper')
            # This is the demonstrated KDE/KWin adapter. Refuse unsupported
            # compositors rather than silently measuring XWayland instead.
            try:
                self.run(['qdbus6', 'org.kde.KWin', '/KWin', 'activeOutputName'])
            except subprocess.CalledProcessError as error:
                raise RuntimeError('Native Wayland automation currently requires KDE/KWin; use --backend x11 explicitly for XWayland') from error
        else:
            self.run(['xdotool', 'getdisplaygeometry'])
        return {'ready': True, 'backend': self.backend, 'display': self.env.get('DISPLAY'),
                'wayland_display': self.env.get('WAYLAND_DISPLAY')}

    def start_input(self):
        if self.backend != 'wayland':
            return
        if self.input_socket.exists():
            raise RuntimeError('Refuse an existing input socket')
        self.input_log = (self.evidence / 'input-daemon.log').open('w')
        self.input_daemon = subprocess.Popen(
            ['ydotoold', '--mouse-off', '--socket-path=' + str(self.input_socket)],
            env=self.env, stdout=self.input_log, stderr=self.input_log)
        (self.evidence / 'input-instance.json').write_text(json.dumps(
            {'pid': self.input_daemon.pid, 'socket': str(self.input_socket)}, indent=2) + '\n')
        deadline = time.monotonic() + 5
        while (not self.input_socket.exists() and self.input_daemon.poll() is None
               and time.monotonic() < deadline):
            time.sleep(0.1)
        if not self.input_socket.exists() or self.input_daemon.poll() is not None:
            raise RuntimeError('Owned Wayland input daemon failed; see input-daemon.log')
        self.env['YDOTOOL_SOCKET'] = str(self.input_socket)
        time.sleep(1)  # Let the compositor register the owned virtual keyboard.

    def alive(self):
        assert self.app.poll() is None, 'Owned application exited'
        assert Path(f'/proc/{self.pid}/exe').resolve() == self.binary, 'Owned executable changed'

    def attach(self, app, pid, title="^Rustleblower$"):
        self.app, self.pid = app, pid
        self.window = None
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            self.alive()
            command = (['kdotool', 'search', '--all', '--pid', str(pid), '--title', title]
                       if self.backend == 'wayland' else
                       ['xdotool', 'search', '--all', '--pid', str(pid)])
            found = self.run(command, check=False)
            # kdotool returns exit 0 with empty output before the surface appears.
            windows = found.stdout.split() if found.returncode == 0 else []
            if self.backend == 'x11':
                assert len(windows) <= 32, 'Refuse an unexpectedly large owned window set'
                # search --name uses legacy WM_NAME on this XWayland stack.
                # getwindowname reads the UTF-8 EWMH name correctly.
                windows = [window for window in windows if re.search(title, self.run(['xdotool', 'getwindowname', window]).stdout.strip())]
            if windows:
                assert len(windows) == 1, 'Refuse an ambiguous owned window'
                self.window = windows[0]
                break
            time.sleep(0.1)
        if self.window is None:
            raise RuntimeError('No owned Rustleblower window appeared')
        command = (['kdotool', 'windowactivate', self.window] if self.backend == 'wayland'
                   else ['xdotool', 'windowactivate', '--sync', self.window])
        self.run(command)

        # Activation acknowledgement can precede actual keyboard focus, notably
        # for a newly mapped XWayland surface. Never send input in that gap.
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                self.assert_focus()
                break
            except (AssertionError, subprocess.CalledProcessError):
                time.sleep(0.1)
        else:
            raise RuntimeError('Owned window did not acquire keyboard focus after activation')

    def assert_focus(self):
        self.alive()
        if self.backend == 'wayland':
            active = self.run(['kdotool', 'getactivewindow']).stdout.strip()
            pid = self.run(['kdotool', 'getwindowpid', self.window]).stdout.strip()
            assert pid == str(self.pid), 'Window ownership changed; refuse input/capture'
        else:
            active = self.run(['xdotool', 'getwindowfocus']).stdout.strip()
        assert active == self.window, 'Focus changed; refuse input/capture'

    def keys(self, *sequence):
        self.assert_focus()
        if self.backend == 'wayland':
            events = [event for key in sequence for event in self.WAYLAND_KEYS[key]]
            self.run(['ydotool', 'key', *events])
        else:
            self.run(['xdotool', 'key', '--window', self.window, '--clearmodifiers', *sequence])

    def type(self, text):
        self.assert_focus()
        command = (['ydotool', 'type', '--key-delay', '20', '--', text]
                   if self.backend == 'wayland' else
                   ['xdotool', 'type', '--window', self.window, '--clearmodifiers', '--delay', '20', text])
        self.run(command)

    def type_url(self, text):
        """Enter fixture URL punctuation without changing the desktop layout."""
        if self.backend != 'wayland':
            self.type(text)
            return
        self.assert_focus()
        index = int(self.run(['qdbus6', 'org.kde.keyboard', '/Layouts',
                              'org.kde.KeyboardLayouts.getLayout']).stdout.strip())
        layouts = self.run(['qdbus6', '--literal', 'org.kde.keyboard', '/Layouts',
                            'org.kde.KeyboardLayouts.getLayoutsList']).stdout
        entries = re.findall(r'\[Argument: \(sss\) "([^"]*)", "([^"]*)", "([^"]*)"\]', layouts)
        if index >= len(entries) or index < 0:
            raise RuntimeError('Cannot establish the active native keyboard layout')
        layout, variant, label = entries[index]
        if (layout, variant) not in [('ca', ''), ('us', '')]:
            raise RuntimeError(f'URL input is not verified for layout {layout}/{variant}')
        if not re.fullmatch(r'[A-Za-z0-9:/._-]+', text):
            raise RuntimeError('Fixture URL contains punctuation not covered by native input')
        (self.evidence / 'input-layout.json').write_text(json.dumps({
            'layout': layout, 'variant': variant, 'label': label,
            'desktop_layout_changed': False, 'slash': 'Shift+3' if layout == 'ca' else '/'
        }, indent=2) + '\n')
        parts = text.split('/') if layout == 'ca' else [text]
        for position, part in enumerate(parts):
            if position:
                self.assert_focus()
                self.run(['ydotool', 'key', '42:1', '4:1', '4:0', '42:0'])
            if part:
                self.type(part)

    def capture(self, path):
        self.assert_focus()
        command = (['spectacle', '--background', '--activewindow', '--nonotify', '--no-decoration', '--no-shadow', '--output', str(path)]
                   if self.backend == 'wayland' else
                   ['magick', 'import', '-limit', 'thread', '1', '-window', self.window, str(path)])
        self.run(command)
        self.assert_focus()

    def close(self):
        try:
            exited = stop_owned_process(self.input_daemon)
            if not exited:
                raise RuntimeError('Owned input daemon remains')
        finally:
            if self.input_log is not None:
                self.input_log.close()
            if (self.input_daemon is not None and self.input_daemon.poll() is not None
                    and self.input_socket.exists()):
                self.input_socket.unlink()
        cleanup = {'owned_input_daemon_exited': exited, 'owned_input_socket_removed': not self.input_socket.exists()}
        (self.evidence / 'input-cleanup.json').write_text(json.dumps(cleanup, indent=2) + '\n')
        return cleanup


def stop_owned_process(app, child_pid=None):
    if child_pid is not None:
        try:
            os.kill(child_pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    if app is not None and app.poll() is None:
        if child_pid is not None:
            try:
                app.stdin.write(b'-gdb-exit\n')
                app.stdin.flush()
            except BrokenPipeError:
                pass
        try:
            app.terminate()
        except ProcessLookupError:
            pass
        try:
            app.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                app.kill()
            except ProcessLookupError:
                pass
            app.wait(timeout=5)
    return ((app is None or app.poll() is not None)
            and (child_pid is None or not Path(f'/proc/{child_pid}').exists()))

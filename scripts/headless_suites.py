#!/usr/bin/env python3
"""Single Kit-journey inventory and Cargo timing extraction."""
import argparse
import re

FEATURE_TESTS = {'browsing': 'browsing::journey',
 'bandwidth': 'bandwidth::journey',
 'settings': 'settings::journey',
 'tray': 'tray_lifecycle::journey',
 'permission-icons': 'permission_icons::journey',
 'permission-actions': 'permission_actions::journey',
 'all': 'user_flow::journey',
 'popup-preferences': 'preferences_controls::journey',
 'countdown': 'countdown_review::journey',
 'checksums': 'checksum_review::journey',
 'history-tools': 'history_tools::journey',
 'history-advanced': 'history_advanced::journey',
 'rules-advanced': 'rules_advanced::journey',
 'updates': 'updates::journey',
 'language': 'language::journey',
 'reputation': 'reputation::journey',
 'raw-downloads': 'raw_downloads::journey',
 'highlighting': 'highlighting::journey',
 'catalogue-policy': 'catalogue_policy::journey',
 'history-selection': 'history_selection::journey',
 'live-table': 'live_table::journey',
 'statistics': 'statistics::journey',
 'nodes': 'nodes::journey',
 'daemon-preferences': 'daemon_preferences::journey',
 'alerts': 'alerts::journey',
 'desktop-notifications': 'desktop_notifications::journey',
 'live': 'live_tasks::journey',
 'firewall': 'firewall::journey',
 'firewall-forms': 'firewall_forms::journey',
 'firewall-files': 'firewall_files::journey',
 'firewall-policy': 'firewall_policy::journey',
 'history-process': 'history_process::journey',
 'history-preferences': 'history_preferences::journey',
 'subscriptions': 'subscriptions::journey',
 'port-ranges': 'port_ranges::journey',
 'cross-node': 'cross_node_copies::journey',
 'multi-node-drafts': 'multi_node_drafts::journey',
 'multi-node-export': 'multi_node_export::journey',
 'startup': 'startup::journey',
 'appearance': 'appearance::journey',
 'window-state': 'window_memory::journey',
 'subscription-url': 'subscription_url_test::journey',
 'subscription-inheritance': 'subscription_inheritance::journey'}

def log_timings(log):
    """Cargo compilation and libtest runtime; absent data stays unknown."""
    text = re.sub(r"\x1b\[[0-9;]*m", "", log)
    build = re.search(r"Finished[^\n]*target\(s\) in ([^\n]+)", text)
    test = re.search(r"test result:[^\n]*finished in ([\d.]+)s", text)
    build_seconds = None
    if build:
        duration = re.fullmatch(r"(?:(\d+)h\s*)?(?:(\d+)m\s*)?(\d+(?:\.\d+)?)s", build[1].strip())
        if duration:
            hours, minutes, seconds = duration.groups()
            build_seconds = int(hours or 0) * 3600 + int(minutes or 0) * 60 + float(seconds)
    return {"build_seconds": build_seconds, "test_seconds": float(test[1]) if test else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    print(" ".join(FEATURE_TESTS))


if __name__ == "__main__":
    main()

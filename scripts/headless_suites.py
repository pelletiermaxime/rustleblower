#!/usr/bin/env python3
"""Single GPU-journey inventory and balanced, isolated CI batches."""
import argparse
from collections import Counter
import re

FEATURE_TESTS = {'browsing': 'browsing::headless_browsing',
 'settings': 'settings_shell::headless_settings',
 'tray': 'headless_tray_lifecycle',
 'permission-icons': 'headless_permission_icons',
 'permission-actions': 'headless_permission_actions',
 'all': 'headless_gpu_user_flow',
 'popup-preferences': 'headless_preferences_controls',
 'countdown': 'headless_countdown_review',
 'checksums': 'headless_checksum_review',
 'history-tools': 'headless_history_tools',
 'history-advanced': 'headless_history_advanced',
 'rules-advanced': 'headless_rules_advanced',
 'updates': 'headless_updates',
 'language': 'headless_language',
 'reputation': 'headless_reputation',
 'raw-downloads': 'headless_raw_downloads',
 'highlighting': 'headless_highlighting',
 'catalogue-policy': 'headless_catalogue_policy',
 'history-selection': 'headless_history_selection',
 'live-table': 'headless_live_table',
 'statistics': 'headless_statistics',
 'nodes': 'headless_nodes',
 'daemon-preferences': 'headless_daemon_preferences',
 'alerts': 'headless_alerts',
 'desktop-notifications': 'headless_desktop_notifications',
 'live': 'headless_live_tasks',
 'firewall': 'headless_firewall',
 'firewall-forms': 'headless_firewall_forms',
 'firewall-files': 'headless_firewall_files',
 'firewall-policy': 'headless_firewall_policy',
 'history-process': 'headless_history_process',
 'history-preferences': 'headless_history_preferences',
 'subscriptions': 'headless_subscriptions',
 'port-ranges': 'headless_port_ranges',
 'cross-node': 'headless_cross_node_copies',
 'multi-node-drafts': 'headless_multi_node_drafts',
 'multi-node-export': 'headless_multi_node_export',
 'startup': 'headless_startup',
 'appearance': 'headless_appearance',
 'window-state': 'headless_window_memory',
 'subscription-url': 'headless_subscription_url_test',
 'subscription-inheritance': 'headless_subscription_inheritance'}

# Balanced by the measured v0.3.0 journeys (including the two repaired flows).
# A batch is serial within its runner; each runner has separate fixture resources.
SHARDS = (('subscriptions',
  'history-selection',
  'history-tools',
  'history-preferences',
  'firewall-policy',
  'catalogue-policy',
  'history-process',
  'startup',
  'permission-icons'),
 ('appearance',
  'reputation',
  'history-advanced',
  'live-table',
  'desktop-notifications',
  'nodes',
  'daemon-preferences',
  'cross-node',
  'firewall',
  'multi-node-export',
  'permission-actions'),
 ('all',
  'firewall-files',
  'subscription-inheritance',
  'statistics',
  'live',
  'updates',
  'settings',
  'window-state',
  'multi-node-drafts',
  'port-ranges',
  'countdown'),
 ('rules-advanced',
  'firewall-forms',
  'language',
  'raw-downloads',
  'highlighting',
  'subscription-url',
  'browsing',
  'alerts',
  'popup-preferences',
  'checksums',
  'tray'))


def validate_shards(shards=SHARDS):
    counts = Counter(feature for shard in shards for feature in shard)
    if not shards or any(not shard for shard in shards):
        raise ValueError("Every GPU batch must contain a journey")
    if set(counts) != set(FEATURE_TESTS) or any(n != 1 for n in counts.values()):
        raise ValueError("GPU batches must cover every journey exactly once")


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
    parser.add_argument("--shard", type=int, choices=range(len(SHARDS)))
    args = parser.parse_args()
    validate_shards()
    print(" ".join(FEATURE_TESTS if args.shard is None else SHARDS[args.shard]))


if __name__ == "__main__":
    main()

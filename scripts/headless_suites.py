#!/usr/bin/env python3
"""Single Kit-journey inventory and balanced, isolated CI batches."""
import argparse
from collections import Counter
import re

FEATURE_TESTS = {'browsing': 'browsing::journey',
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

# Fixed isolated batches covering every Kit journey once.
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
        raise ValueError("Every Kit batch must contain a journey")
    if set(counts) != set(FEATURE_TESTS) or any(n != 1 for n in counts.values()):
        raise ValueError("Kit batches must cover every journey exactly once")


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

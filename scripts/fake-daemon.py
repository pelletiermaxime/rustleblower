#!/usr/bin/env python3
"""Exercise Rustleblower on an isolated socket using OpenSnitch's installed Python bindings.

Only synthetic connections to documentation addresses are reported; no firewall is changed.
Requires the existing opensnitch-ui Python package (grpc + protobuf).
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
from collections import Counter
import json
import queue
from pathlib import Path
import signal
import threading
import time

import grpc
from opensnitch.proto import ui_pb2 as proto, ui_pb2_grpc
from google.protobuf.json_format import MessageToDict


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--socket", required=True, type=Path)
    parser.add_argument("--expected-duration", default="once", choices=["once", "12h", "always"])
    parser.add_argument("--authority", help="Optional gRPC authority override; default tests natural UDS behavior")
    parser.add_argument("--events", type=int, default=430)
    parser.add_argument("--alerts",type=int,default=0,choices=range(65),metavar="0..64",help="Report bounded synthetic SHOW_ALERT messages")
    parser.add_argument("--prompts", type=int, default=2)
    parser.add_argument("--checksum-rules", type=int, default=0, choices=range(0, 101), metavar="0..100")
    parser.add_argument("--rules", type=int, default=6, help="Synthetic application rules in the subscription inventory")
    parser.add_argument("--allow-live-tasks", action="store_true", help="Synthetic task results only; never reads real processes or sockets")
    parser.add_argument("--allow-node-config", action="store_true", help="Acknowledge configuration in this synthetic client only; never writes system configuration")
    parser.add_argument("--system-firewall-drop",action="store_true",help="Use a drop policy only in the owned in-memory system-firewall fixture")
    parser.add_argument("--allow-system-firewall", action="store_true", help="Report and acknowledge an in-memory synthetic system firewall; never invokes a host firewall")
    parser.add_argument("--reject-rule-changes", action="store_true", help="Exercise the UI's daemon rejection path")
    parser.add_argument("--burst-batches", type=int, default=0)
    parser.add_argument("--batch-size", type=int, default=256)
    parser.add_argument("--seconds", type=int, default=120)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args()
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    signal.signal(signal.SIGINT, lambda *_: stop.set())
    options = [("grpc.default_authority", args.authority)] if args.authority else []
    channel = grpc.insecure_channel("unix:" + str(args.socket.resolve()), options=options)
    grpc.channel_ready_future(channel).result(timeout=10)
    client = ui_pb2_grpc.UIStub(channel)
    rules = {f"fixture.application.{i:03}": proto.Rule(
        name=f"fixture.application.{i:03}", description=f"Synthetic application {i}",
        enabled=True, action="deny" if i % 2 == 0 else "allow", duration="always" if i % 3 else "5m",
        operator=proto.Operator(type="simple", operand="process.path", data=f"/usr/bin/synthetic-app-{i}", sensitive=True)
    ) for i in range(args.rules)}
    for index in range(args.checksum_rules):
        conditions = [proto.Operator(type="simple", operand="process.path", data="/usr/bin/synthetic-app-1", sensitive=True),
                      proto.Operator(type="simple", operand="process.hash.md5", data="c" * 32),
                      proto.Operator(type="simple", operand="process.hash.sha1", data="d" * 40)]
        rules[f"fixture.checksum.{index:03}"] = proto.Rule(name=f"fixture.checksum.{index:03}",
            description="Synthetic checksum rule", enabled=True, action="deny", duration="always",
            operator=proto.Operator(type="list", operand="list", data=json.dumps([{"type": op.type, "operand": op.operand, "data": op.data, "sensitive": op.sensitive} for op in conditions]), list=conditions))
    config = proto.ClientConfig(name="Synthetic local daemon", version="synthetic-kernel", isFirewallRunning=True, config='{"DefaultAction":"allow"}', rules=list(rules.values()))
    if args.allow_system_firewall:
        config.config=json.dumps({"DefaultAction":"allow","Firewall":"nftables"})
        config.systemFirewall.Enabled=True
        config.systemFirewall.Version=1
        chain=config.systemFirewall.SystemRules.add().Chains.add(Name="fixture-input",Table="filter",Family="inet",Type="filter",Hook="input",Policy="drop" if args.system_firewall_drop else "accept")
        chain.Rules.add(UUID="fixture-system-rule",Enabled=True,Description="Synthetic system rule",Target="accept")
    sequence = 0
    started = time.monotonic()
    counters = {name: Counter() for name in ("by_executable", "by_host", "by_address", "by_port", "by_uid", "by_proto")}

    def ping(count):
        nonlocal sequence
        now = time.time_ns()
        events = []
        for i in range(count):
            sequence += 1
            events.append(proto.Event(unixnano=now + i, connection=proto.Connection(
                process_path=["/usr/bin/firefox", "/usr/bin/curl", "/usr/bin/ssh"][sequence % 3],
                dst_host="api.example.test", dst_ip="192.0.2.42", dst_port=443,
                protocol="tcp", user_id=1000, process_id=4242),
                rule=proto.Rule(action="deny" if sequence % 3 == 0 else "allow", name="fixture")))
        for event in events:
            connection = event.connection
            for name, key in (("by_executable", connection.process_path), ("by_host", connection.dst_host),
                              ("by_address", connection.dst_ip), ("by_port", str(connection.dst_port)),
                              ("by_uid", str(connection.user_id)), ("by_proto", connection.protocol)):
                counters[name][key] += 1
        start = time.monotonic()
        response = client.Ping(proto.PingRequest(id=sequence, stats=proto.Statistics(
            daemon_version="1.7.2 (synthetic)", events=events, uptime=int(time.monotonic() - started),
            connections=sequence, accepted=sequence - sequence // 3, dropped=sequence // 3, **counters)), timeout=1)
        assert response.id == sequence
        return (time.monotonic() - start) * 1000

    # The real daemon starts Subscribe asynchronously and may report traffic
    # first. Exercise that ordering every time instead of masking the race.
    timings = [ping(args.events)]
    assert client.Subscribe(config, timeout=2).config == config.config
    for number in range(args.alerts):
        response=client.PostAlert(proto.Alert(id=number,type=proto.Alert.WARNING,action=proto.Alert.SHOW_ALERT,priority=proto.Alert.HIGH,text=f"Synthetic daemon alert {number:03}"),timeout=1)
        assert response.id==number
    acknowledgements = queue.Queue(maxsize=16)
    commands = []

    def replies():
        while not stop.is_set():
            try:
                yield acknowledgements.get(timeout=0.25)
            except queue.Empty:
                continue

    stream = client.Notifications(replies())

    def receive_notifications():
        try:
            for notification in stream:
                if args.allow_live_tasks and notification.type in (proto.TASK_START, proto.TASK_STOP):
                    task_config = json.loads(notification.data)
                    assert task_config["name"] == "sockets-monitor", "This native fixture only supports socket inspection"
                    if len(commands) < 1024:
                        commands.append({"id":notification.id, "type":notification.type, "task":task_config, "acknowledged":False})
                    if notification.type == proto.TASK_START:
                        table = [{"PID":42, "Proto":6, "Iface":"eth0", "Socket":{"ID":{"Source":"192.0.2.1", "Destination":"198.51.100.2", "SourcePort":30000+i, "DestinationPort":443, "Cookie":[1,2]}, "Family":2, "State":1, "UID":1000, "INode":5000+i, "RQueue":7, "WQueue":9}} for i in range(3)]
                        payload = json.dumps({"Table":table,"Processes":{"42":{"Path":"/usr/bin/synthetic-live", "Comm":"synthetic-live"}}})
                        acknowledgements.put(proto.NotificationReply(id=notification.id, code=proto.OK, data=payload), timeout=1)
                    # Pinned daemon has no successful TASK_STOP response.
                    continue
                code, data = proto.OK, ""
                if args.reject_rule_changes:
                    code, data = proto.ERROR, "Synthetic daemon rejected the rule change"
                elif notification.type == proto.CHANGE_CONFIG and args.allow_node_config:
                    try:
                        proposed = json.loads(notification.data)
                        if not isinstance(proposed, dict):
                            raise ValueError("Configuration must be an object")
                        config.config = notification.data
                    except (ValueError, TypeError) as error:
                        code, data = proto.ERROR, str(error)
                elif notification.type == proto.RELOAD_FW_RULES and args.allow_system_firewall:
                    if not notification.HasField("sysFirewall") or notification.sysFirewall.Version!=1 or notification.sysFirewall.ByteSize()>256*1024:
                        code,data=proto.ERROR,"Invalid synthetic system-firewall payload"
                    else:
                        config.systemFirewall.CopyFrom(notification.sysFirewall)
                elif notification.type not in (proto.CHANGE_RULE, proto.ENABLE_RULE, proto.DISABLE_RULE, proto.DELETE_RULE):
                    code, data = proto.ERROR, "Unsupported synthetic command"
                else:
                    for item in notification.rules:
                        if notification.type == proto.DELETE_RULE:
                            rules.pop(item.name, None)
                        else:
                            saved = proto.Rule()
                            saved.CopyFrom(item)
                            if notification.type in (proto.ENABLE_RULE, proto.DISABLE_RULE):
                                saved.enabled = notification.type == proto.ENABLE_RULE
                            rules[item.name] = saved
                if len(commands) < 1024:
                    commands.append({"id": notification.id, "type": notification.type,
                                     "rules": [r.name for r in notification.rules], "code": code,
                                     "configuration": json.loads(notification.data) if notification.type == proto.CHANGE_CONFIG and code == proto.OK else None,
                                     "system_firewall": MessageToDict(notification.sysFirewall,preserving_proto_field_name=True) if notification.type==proto.RELOAD_FW_RULES else None})
                acknowledgements.put(proto.NotificationReply(id=notification.id, code=code, data=data), timeout=1)
        except grpc.RpcError:
            if not stop.is_set():
                raise

    threading.Thread(target=receive_notifications, daemon=True).start()
    results = []

    def ask(i):
        start = time.monotonic()
        rule = client.AskRule(proto.Connection(process_path=f"/usr/bin/synthetic-app-{i + 1}",
            dst_ip="192.0.2.42", dst_host="permission.example.test", dst_port=443,
            protocol="tcp", user_id=1000, process_id=5000 + i,
            process_checksums={"process.hash.md5": "a" * 32, "process.hash.sha1": "b" * 40} if args.checksum_rules else {}), timeout=120)
        result = {"prompt": i + 1, "action": rule.action, "duration": rule.duration,
                  "seconds": round(time.monotonic() - start, 3)}
        assert rule.duration == args.expected_duration
        results.append(result)
        print(json.dumps(result), flush=True)

    with ThreadPoolExecutor(max_workers=max(1, args.prompts)) as pool:
        futures = [pool.submit(ask, i) for i in range(args.prompts)]
        for _ in range(args.burst_batches):
            timings.append(ping(args.batch_size))
        deadline = time.monotonic() + args.seconds
        while time.monotonic() < deadline and not stop.wait(0.25):
            timings.append(ping(0))
        for future in futures:
            future.result()
    report = {"events_sent": sequence, "ping_max_ms": round(max(timings), 3),
              "ping_count": len(timings), "decisions": sorted(results, key=lambda r: r["prompt"]),
              "commands": commands,
              "rules": [{"name": r.name, "action": r.action, "enabled": r.enabled, "duration": r.duration} for r in rules.values()]}
    if args.report:
        args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)
    stop.set()
    stream.cancel()
    channel.close()


if __name__ == "__main__":
    main()

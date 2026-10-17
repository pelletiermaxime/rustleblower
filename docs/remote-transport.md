# TCP, TLS and mutual TLS server transport

Rustleblower remains the OpenSnitch UI **server**. The daemon connects to it;
the daemon still owns policy/enforcement. Default launch binds the same absolute
Unix socket path and never unlinks an existing socket. Owned socket cleanup
continues to check its device/inode before removing the path.

Listener forms are an absolute path, `unix:///absolute/path`, a numeric
`IPv4:port`, `[IPv6]:port`, or an optional `tcp://` prefix. Listener hostnames,
abstract Unix sockets and TLS-over-Unix are outside this slice. Network address
configuration does not edit/restart the daemon or create a firewall rule.

```sh
rustleblower --socket 127.0.0.1:50051
rustleblower --socket '[::]:50051' --socket-auth tls-simple \
  --tls-cert /absolute/server.pem --tls-key /absolute/server.key
rustleblower --socket '[::]:50051' --socket-auth tls-mutual \
  --tls-ca-cert /absolute/ca.pem \
  --tls-cert /absolute/server.pem --tls-key /absolute/server.key
```

These modes come from the pinned source contract at
`a1353848ba1b660320e90cefea782c3fba272c00`:
`ui/bin/opensnitch-ui:137-177,303-332`,
`ui/opensnitch/auth/__init__.py:25-39`, and
`daemon/ui/auth/auth.go:26-97`. Plain mode uses the existing unencrypted gRPC
protocol. TLS-simple presents the configured server identity. TLS-mutual also
requires a client certificate verified by the configured CA. Rustleblower rejects
a CA flag in TLS-simple mode instead of silently enabling client authentication;
choose TLS-mutual explicitly. Invalid/missing credentials fail startup; no mode
falls back to plaintext.

PEM reads happen once before the GPUI application starts. Each file is regular,
opened with no-follow/nonblocking flags and limited to 64 KiB. Symlink/FIFO/device,
empty and oversized inputs are refused. Secrets never enter startup entries:
autostart preserves the numeric endpoint, auth mode and absolute credential paths
only. Relative path arguments become absolute when saving the startup entry.
Certificate contents are not copied into preferences or logs.

## Identity and local-only functionality

Transport peer addresses determine locality; daemon-reported names and JSON do
not. Unix peers are local. TCP uses normalized IPv4/IPv6/mapped addresses and a
bounded startup snapshot of local interfaces, with loopback recognized directly.
A TCP cache/session key is `peer-IP/reported-name`, omitting the ephemeral source
port so reconnects are stable. Keys retain the existing 128-byte bound; names
that cannot fit alongside the peer address are refused. Separate IPs with equal
reported names do not replace each other or a Unix key. Multiple daemons behind
one source address still need distinct configured names. This is socket/session
identity; the self-reported name is not a certificate identity.

IPv6 zone/scope IDs are retained in TCP keys (`fe80::1%2/name`), so equal
link-local addresses on separate interfaces do not collide. Link-local TCP is
conservatively classified remote because the unscoped startup IP snapshot cannot
prove its zone-local identity; loopback, global IPv6 and mapped IPv4 keep their
normal classification. Local subscriptions should use Unix/loopback transport
when link-local routing is involved.

Automatic local subscription updates require a current local notification stream.
Single-list and collection rule creation/inspection reject a selected remote
session because their file paths and UID scope belong to this host. Existing
node-targeted rule/configuration commands continue using captured sessions and
the shared confirmed command path. Interface addresses are snapshotted at startup;
restart after a network-interface/address change to refresh the classification.

## Admission and resource boundaries

All connections retain one of 64 lifetime permits, acquired before Tonic can
allocate a TLS-handshake task. TCP additionally requires one of eight initial
permits until successful Subscribe. A ten-second initial I/O deadline covers TLS
and a missing Subscribe; failures drop their permits. Established connections
continue independently of the initial pool. Every HTTP/2 connection permits at
most 64 concurrent streams. There is no connection-admission waiter queue.

Before generated protobuf decoding, a native Tower wrapper recognizes only the
five canonical UI RPC paths and uses separate shared nonwaiting pools:

| RPC | Active request/header capacity | Reason |
| --- | --- | --- |
| AskRule | 40 | Existing 32 pending requests plus eight slots for prompt overload/fallback handlers. |
| Ping | 4 | Telemetry decoding cannot take permission/Subscribe slots. |
| Subscribe | 2 | Large inventories have a separate small admission pool. |
| PostAlert | 4 | Alert decoding cannot take permission or Ping slots. |
| Notifications | 4 until headers | Fast stream setup; the existing one-stream-per-session/32-node gate bounds retained incoming streams after headers. |

Method permits remain with the response future and are released on response,
failure or cancellation; excess requests return gRPC ResourceExhausted without
protobuf decoding or an allocated waiter. Unknown methods return Unimplemented
before decoding. The existing 32-prompt once-rule fallback still handles admitted
AskRule overflow; overload beyond the transport's 40 admitted requests is a
transport error, not an invented policy reply. Existing ingestion, persistence,
payload and command limits remain in force.

The 54 active unary/header slots times the 2-MiB message cap permit **108 MiB of
encoded decoder input** in a deliberately pessimistic simultaneous workload.
The at-most-32 established notification streams can separately decode one bounded
message each. This arithmetic is not an RSS bound: protobuf repeated-field/string
allocation, buffer capacities, HTTP/2 framing/windows, queued ingestion, SQLite,
TLS and GPU memory add overhead or decoded-size amplification. Dedicated pools
isolate different methods; they do not establish adversarial same-method fairness
or a measured latency guarantee. Long-duration RSS and busy-traffic measurements
remain required before claiming strong low-memory/performance proof.

## Verification contract

`tests/remote_transport.rs` covers endpoint/auth parsing, socket-derived locality
and keys, startup argument preservation, invalid credential refusal, actual
plaintext/TLS/mTLS requests, wrong CA/name/plaintext/missing/untrusted client
refusal, successful listener reuse after failure, ConnectInfo routing, held ACK
with independent permission and Ping, reconnect identity, initial capacity/deadline,
and nonwaiting method pool isolation/cancellation. Network cases refuse to run
outside the owned mapped-user/loopback namespace:

```sh
python scripts/owned_net.py --evidence /absolute/new/evidence/network.json -- \
  python scripts/cargo.py test --no-default-features --locked --test remote_transport
```

Retain exact command/log/result/network evidence. Constructor/source review and a
green core compile are not actual TLS proof. Tonic-client interoperability,
the exact pinned Go daemon client, real remote host authentication, packaged
desktop behavior and enforcement are distinct proof layers. This slice does not
change the running daemon, adopt its socket or claim kernel enforcement.

The complete owned-namespace core checkpoint passed 200 cases before the narrow
IPv6 scope-key refinement and concurrent missed-notice changes. The final
focused transport/helper run passes six transport and five helper cases, with
exact source hashes and log digest in
`.verification/remote-transport-20261016/final-focused-result.json`. No exact
grpc-go fixture was fetched: that pinned client's dependencies are unavailable
offline, so its distinct compatibility layer remains unproved.

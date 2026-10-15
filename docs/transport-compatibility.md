# OpenSnitch Unix-socket compatibility

Rustleblower applies `patches/h2-opensnitch.patch` to the unmodified h2 0.4.20
release. Upstream source is generated under ignored `.deps/h2-0.4.20`, never
checked into this project. The extracted package retains its upstream MIT
license. The patch adds the opt-in `opensnitch-uds-authority` feature.

OpenSnitch 1.7.2 uses gRPC-Go 1.32.0 with a custom Unix dialer and a bare socket
path target. Although this grpc-go version supports a localhost authority for
`unix:` targets, OpenSnitch's bare target bypasses that handling. It puts the
path in `:authority`; stock h2 rejects it before tonic sees the request, producing
RST_STREAM/PROTOCOL_ERROR. Newer gRPC C-core clients also send percent-encoded
socket-path authorities.

The patch changes only the server's URI conversion. An otherwise-invalid
authority that looks like a Unix socket path is normalized to `localhost` only
for the five exact OpenSnitch UI RPCs. Valid authorities, other request paths,
CONNECT, and unrelated malformed hosts retain normal validation. Rustleblower
listens only on Unix sockets and does not authenticate or route by authority.
Revisit this policy before adding TCP serving. No proxy process, buffering layer,
or runtime dependency is added.

## Preparing the dependency

Use `python scripts/cargo.py` in place of `cargo` to prepare the dependency before
Cargo resolves its manifest. Alternatively, run
`python scripts/prepare_dependencies.py` once, then use Cargo normally. Python
3.12 or newer and the `patch` command are required.

The preparation script reuses an archive in Cargo's cache when available;
otherwise it downloads the exact release from static.crates.io. It checks the
pinned SHA-256 before extraction and applies the diff with no fuzz. An input
stamp and hashes of the patched files avoid repeating preparation. `--offline`
uses only verified cached archives; the Cargo wrapper also honors `--frozen`.
Deleting `.deps/` is safe when builds are stopped; the next wrapped command
recreates it. A normal Cargo build cannot recreate a missing path dependency
through the application's `build.rs`, because resolution happens first.

We evaluated cargo-patch 0.3.2: its locked installation fails on our Rust
toolchain in time 0.3.23 (E0282). It also embeds Cargo 0.72, predating our edition
2024 manifest. The small preparation script avoids adding that tool and its
dependency graph to every developer's build.

## Updating and checking

Keep the h2 version in Cargo.toml, its path override, and the script's VERSION
and ARCHIVE_SHA256 together. Regenerate the diff against the exact new release,
review it, and run the locked protocol tests. Never edit generated `.deps/`
source as a permanent fix; change the patch file instead.

`tests/uds_authority.rs` sends raw HTTP/2 frames for accepted socket-path
authorities and rejected unrelated paths/hosts. The Python fixture uses its
natural UDS authority; forcing localhost would mask this compatibility failure.
The background GPU and owned native verification flows use the same production
transport. An upstream configurable conversion hook would let us remove the
patch and preparation step.

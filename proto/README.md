# Rustleblower wire contract

`rustleblower.proto` is Rustleblower's MIT-licensed interoperability schema for
the OpenSnitch UI service. Its numeric tags, scalar types, enum values, package
and RPC paths match the installed 1.7.2 daemon interface. Field spelling and
source ordering are local choices; protobuf compatibility depends on the wire
contract. Upstream implementation code and explanatory comments are not included.
OpenSnitch is a separate GPL-licensed program and is not bundled in this project.

The interoperability reference is the daemon's public
[UI interface](https://github.com/evilsocket/opensnitch/blob/v1.7.2/proto/ui.proto).
The initial imported generated file was replaced before Rustleblower's first
published commit/release. Current `src/protocol.rs` is generated exclusively from
this local schema with the Prost (Apache-2.0) and Tonic (MIT) generators.

Normal builds use the checked-in Rust file and do not need `protoc`. To regenerate
intentionally, install `protoc`, then run from the repository root:

```sh
cargo run --manifest-path tools/protocol-generator/Cargo.toml --locked
```

The generator has its own lockfile and exact tonic-build 0.12.3 / prost-build
0.13.5 versions. Keep wire tags and RPC streaming directions stable; run core
transport tests and the installed OpenSnitch Python-client fixture after changes.

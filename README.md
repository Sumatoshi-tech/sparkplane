# Sparkplane

An independent DGX Spark inference appliance and workstation CLI. Sparkplane
owns model downloads, declarative engine profiles, authenticated OpenAI and
Anthropic gateways, signed releases, resource admission and GPU qualification.
It does not depend on sy, a desktop session, an AMD NPU runtime or a Sparky checkout.

```sh
cargo build --locked --release
./target/release/sparkplane --help
./target/release/sparkplane dgx-spark status --json
./target/release/sparkplane dgx-spark serve qwen3.8:flash-next --dry-run --json
```

Use the pinned Rust 1.95.0 toolchain (selected automatically by rustup), a
C/C++ toolchain, CMake, pkg-config and Python 3.11+.
The client runs on Linux x86-64/ARM64; the appliance targets Linux ARM64 with
NVIDIA GB10, Docker and the NVIDIA container toolkit already installed.

## Architecture

The unprivileged HTTPS agent owns state and protocol translation. A separate,
root-owned executor accepts only typed operations over peer-authorized local
IPC. Only the executor can contact Docker. Engines run on an internal network;
native tokenizer, health and administrative endpoints are not public.

The default build is client-only. `--features appliance` adds the agent,
executor, database, gateway and bootstrap implementation. The owned
`sparkplane-core` and `sparkplane-ipc` crates contain only platform vocabulary,
trace propagation, notification and bounded IPC.

```sh
make lint
make test
make test-client
make audit
cargo build --locked --release --features appliance
```

Three inherited hardware/client qualification tests require explicit external
fixtures and are ignored by default. Ordinary unit and integration tests are
hermetic and do not contact a Spark or allocate a GPU.

## Documentation

- [Build, release, deploy, add engines and models](docs/how-to/develop-spark.md)
- [Install, upgrade and rollback](docs/how-to/install-spark.md)
- [Extraction and compatibility contracts](docs/extraction.md)
- [Legacy migration status and safety gates](docs/how-to/migrate-from-sy.md)
- [Security and release authority](SECURITY.md)

Inference URLs stay at `/openai/<instance>/v1` and `/anthropic/<instance>/v1`.
The control API uses `/api/sparkplane/v1`. Runtime paths, units, schemas,
environment variables and Docker ownership use the Sparkplane namespace.

The companion `sy spark HOST ...` bridge forwards to a pinned, verified client.
Sparkplane itself requires no sy installation. Sparky is a separate research
project whose engine artifacts use the same signed qualification interface.

## Provenance

Extracted from the sy working tree on 2026-09-21, including the optimized vLLM
GPU device mapping fix. `PROVENANCE.json` records the original source hashes.
Content-addressed engine patches retain their original bytes and attribution;
historical names inside those immutable payloads are not active service names.
The original MIT license is retained.

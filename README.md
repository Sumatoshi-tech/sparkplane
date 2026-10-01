# Sparkplane

Run and manage language models on NVIDIA DGX Spark. Sparkplane provides a
workstation CLI, model downloads, declarative engine profiles, authenticated
OpenAI and Anthropic gateways, signed releases, resource admission and GPU
qualification.

```sh
cargo build --locked --release
./target/release/sparkplane --help
./target/release/sparkplane dgx-spark status --json
./target/release/sparkplane dgx-spark serve qwen3.8:flash-next --dry-run --json
./target/release/sparkplane dgx-spark launch codex
./target/release/sparkplane webui
```

Use the pinned Rust 1.95.0 toolchain (selected automatically by rustup), a
C/C++ toolchain, CMake, pkg-config and Python 3.11+.
Install the Minisign executable for the encrypted release-signing tests.
The client runs on Linux x86-64/ARM64; the appliance targets Linux ARM64 with
NVIDIA GB10, Docker and the NVIDIA container toolkit already installed.

Agent launches default to `--mode auto`: full filesystem and network access
without action approval prompts. Use `--mode inherit` to keep the agent's own
permission settings. In inherit mode, `--allow-network` grants network access
within a supported sandbox. See [launch permissions](docs/reference/spark.md#internet-access-for-development).

Launches also default to `--eco-mode=max`, using RTK filters embedded in the
Sparkplane executable. Use `--eco-mode=none` to disable compression. When the
coding session exits, Sparkplane prints token usage and the same model's public
cloud-equivalent cost when a verified tariff exists; `sparkplane <host> economics
--json` reads the latest retained report.
See [session economics](docs/reference/spark.md#eco-mode-and-session-economics).

Vision launches also retain recent images automatically and archive older
originals for reopening. A private local adapter prunes historical image payloads
before upload, while the gateway enforces the qualified model limits. See
[image history and gateway limits](docs/reference/spark.md#image-history-and-upload-limits).

## Architecture

The unprivileged HTTPS agent owns state and protocol translation. A separate,
root-owned executor accepts only typed operations over peer-authorized local
IPC. Only the executor can contact Docker. Engines run on an internal network;
native tokenizer, health and administrative endpoints are not public.

The default build is client-only. `--features appliance` adds the agent,
executor, database, gateway and bootstrap implementation. The owned
`sparkplane-core` and `sparkplane-ipc` crates contain only platform vocabulary,
trace propagation, notification and bounded IPC.

The web panel uses shadcn/ui with a black and NVIDIA-green theme. Run
`sparkplane webui` to open it and click **Authenticate**, using your existing
CLI credential. The local authentication service stops after login. If you
open `/panel/` directly, run `sparkplane webauthsvc start` on the browser's
computer. See [web panel](docs/reference/spark.md#web-panel).

Appliance builds additionally need Node.js 22.12+ and npm to compile the panel:
run `make web-build` before invoking Cargo with `--features appliance`.
Compiled assets are embedded in the signed binary; Spark needs no Node runtime.

```sh
make lint
make test
make test-client
make audit
make web-test-browser # first: cd web && npx playwright install chromium
cargo build --locked --release --features appliance
```

Hardware, client and privilege-dependent tests require explicit external
fixtures and are ignored by default. Ordinary unit and integration tests are
hermetic and do not contact a Spark or allocate a GPU.

## Documentation

- [Build, release, deploy, add engines and models](docs/how-to/develop-spark.md)
- [Install, upgrade and rollback](docs/how-to/install-spark.md)
- [CLI and runtime reference](docs/reference/spark.md)
- [Security and release authority](SECURITY.md)

To build and preview a signed source update, run `make update HOST=dgx-spark`.
Apply it with `python3 scripts/update-spark.py dgx-spark --apply`; see the
[workstation update workflow](docs/how-to/develop-spark.md#update-a-spark-from-this-checkout).

Inference URLs use `/openai/<instance>/v1` and `/anthropic/<instance>/v1`.
The control API uses `/api/sparkplane/v1`. Runtime paths, units, schemas,
environment variables and Docker ownership use the Sparkplane namespace.

## License

[MIT](LICENSE).

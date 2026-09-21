# Develop and release the Spark plane

This guide is for contributors who change the Spark agent, its engine and
model catalogs, or the ARM64 release that runs on a DGX Spark. It describes
the supported path from a source change to a verified deployment.

## What is deployed

The Spark release is intentionally small. The signed bundle contains:

- `sparkplane-aarch64`, the ARM64 `sparkplane` executable built with the `appliance` feature;
- `configs/sparkplane/models.toml`, the model catalog;
- every `configs/sparkplane/engines/*.toml`, the engine catalog; and
- `SHA256SUMS`, plus `SHA256SUMS.minisig` for a tagged release.

The bundle does not contain Docker images or model weights. Engine images are
referenced by immutable digest and must already be available through the
configured registry or local image transport. Model snapshots are acquired on
Spark by the managed download operation and are addressed by repository and
commit revision.

The laptop is the control plane. `sparkplane <host> ...` talks to the remote
agent over its pinned TLS endpoint; it does not run Docker commands or copy
arbitrary files over SSH. SSH/SFTP is used only by the signed bootstrap and
upgrade protocol.

## Repository map

| Area | Purpose |
| --- | --- |
| `.github/workflows/spark-release.yml` | CI contract, ARM64 build, packaging, and tagged signing |
| `scripts/package-spark-release.sh` | Creates the exact release bundle and checksum manifest |
| `configs/sparkplane/agent.toml`, `executor.toml` | Listener, paths, resource reserves, and pinned host policy |
| `configs/sparkplane/engines/*.toml` | Digest-pinned runtime recipes, matchers, routes, and profiles |
| `configs/sparkplane/models.toml` | Reproducible model identities and artifact metadata |
| `src/spark/` | Catalog parsing, admission, lifecycle, IPC, install, and upgrade logic |
| `tests/spark_release_catalog_boundary.rs` | Boundary tests for every shipped catalog entry |
| `docs/how-to/install-spark.md` | Operator installation, upgrade, rollback, and certificate rotation |
| `docs/how-to/serve-a-model-on-spark.md` | Operator model download and serving walkthrough |
| `docs/reference/spark.md` | CLI and runtime contracts |

## Local development loop

Run the same checks that gate the release before opening a pull request:

```bash
make lint
make test
make test-client
make audit
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps
```

For a catalog-only change, the focused boundary test is useful:

```bash
cargo test --test spark_release_catalog_boundary --no-default-features --features appliance
```

The tests parse and validate the real files under `configs/`; do not replace
them with a second test fixture. A new workload or protocol path also needs a
black-box test against the running daemon where practical.

## Build the ARM64 executable

Install the target and the tools used by CI (`cargo-zigbuild` is preferred for
the cross linker):

```bash
rustup target add aarch64-unknown-linux-gnu
cargo install cargo-auditable cargo-zigbuild
python3 -m pip install --user ziglang==0.16.0
```

Build locally with the same feature boundary as Spark:

```bash
cargo zigbuild --release \
  --target aarch64-unknown-linux-gnu \
  --no-default-features --features appliance --bin sparkplane
```

CI uses `cargo auditable zigbuild` so the resulting binary carries its Rust
dependency inventory. Use the `appliance` feature for the ARM64 service binary
and the default feature set for workstation clients.

Install the [Minisign executable](https://jedisct1.github.io/minisign/) separately
for signing and the encrypted-key release tests; the Rust `minisign` crate is a
library, not that executable.

The Rust toolchain is pinned by `rust-toolchain.toml`. Zig is a separate
requirement of [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild#installation);
the release workflow installs it explicitly. Keep local and CI versions aligned.
Every CI toolchain setup, including cross-build jobs, explicitly installs the
components from `rust-toolchain.toml`. This avoids a second implicit toolchain
installation when Cargo first runs in the checkout.

## Package and sign a release

Run `make lint` for warning-denying checks on both client and appliance builds.
`make lint-spark` is an alias for the same gate.

Package the binary and catalogs from the repository root:

```bash
scripts/package-spark-release.sh \
  target/aarch64-unknown-linux-gnu/release/sparkplane \
  target/spark-release
```

The script fails if the engine catalog is empty and writes a manifest covering
the binary, model catalog, and every engine TOML. Verify it before signing:

```bash
(cd target/spark-release && sha256sum -c SHA256SUMS)
minisign -Sm target/spark-release/SHA256SUMS -s /secure/path/sparkplane-release.key
minisign -V -m target/spark-release/SHA256SUMS \
  -x target/spark-release/SHA256SUMS.minisig \
  -p /secure/path/sparkplane-release.pub
```

The secret key and password belong in a password manager or CI secret. Never
commit them, place them in the bundle, or print them in a log. The public key
is pinned in the operator's local Spark configuration. A manually dispatched
workflow builds and uploads an artifact but deliberately does not sign it;
only a `v*` tag invokes the CI signing step.

### Provision the release authority

Generate the dedicated encrypted key in your own terminal. Store its password
in your password manager; do not send it in chat or put it in a command argument.
The commands below do not overwrite an existing key pair:

```bash
sparkplane_signing_dir="${XDG_CONFIG_HOME:-$HOME/.config}/sparkplane-release-signing"
install -d -m 700 "$sparkplane_signing_dir"
minisign -G -p "$sparkplane_signing_dir/release.pub" \
  -s "$sparkplane_signing_dir/release.key"
gh secret set SPARKPLANE_MINISIGN_SECRET_KEY \
  --repo Sumatoshi-tech/sparkplane --env release < "$sparkplane_signing_dir/release.key"
gh secret set SPARKPLANE_MINISIGN_PASSWORD \
  --repo Sumatoshi-tech/sparkplane --env release
```

The last command prompts for the same password interactively. Back up the key
securely and distribute the public key through your established trust channel.
The protected `release` environment must already exist with maintainer review
and `v*` tag-only deployment rules. Neither PR checks nor unsigned build jobs
receive these secrets. Never replace an installed authority merely because a
new public key accompanies a download.

## CI/CD lifecycle

The release workflow runs on `workflow_dispatch` and on tags matching
`v*`:

1. Check dependencies, tests, clippy, and formatting.
2. Build auditable x86-64 and ARM64 clients, plus the ARM64 appliance.
3. Package the appliance and all catalogs with an inner `SHA256SUMS`.
4. On a `v*` tag, sign that inventory in the protected `release` environment,
   archive the appliance, and sign an outer inventory covering all downloads.
5. Publish both clients, `sparkplane-appliance-aarch64.tar.gz`, and the outer
   inventory/signature as GitHub release assets. Manual dispatch uploads
   `sparkplane-<target>` workflow artifacts without publication.

There is no automatic production deploy from GitHub Actions. An operator
downloads the artifact, verifies the signature, and explicitly invokes
`sparkplane`. This keeps a failed engine start from silently changing a live
Spark and leaves the host fingerprint and release key under local control.

For a tagged release, publish the matching public key and record the tag,
commit, artifact name, and manifest signature in the release notes. The
artifact must be the one whose manifest was signed; do not rebuild it after
signing.

Installation assigns explicit permissions independently of the operator's
umask. Public release directories are root-owned and traversable by the service
account; executable and catalog payloads remain read-only. Private credentials,
state and rollback evidence retain their restricted modes.

## Install or upgrade Spark

For a new host, use the bootstrap installer. Always inspect the dry run first:

```bash
sparkplane dgx-spark install --dry-run --json
sparkplane dgx-spark install --yes \
  --probe release/sparkplane-aarch64 \
  --release-manifest release/SHA256SUMS \
  --release-signature release/SHA256SUMS.minisig \
  --release-public-key sparkplane-release.pub
```

For an installed host, use the side-by-side upgrade protocol:

```bash
sparkplane dgx-spark upgrade --dry-run --json
sparkplane dgx-spark upgrade --yes \
  --probe release/sparkplane-aarch64 \
  --release-manifest release/SHA256SUMS \
  --release-signature release/SHA256SUMS.minisig \
  --release-public-key sparkplane-release.pub
```

The installer verifies the signed manifest, the pinned DGX host fingerprint,
and the content-addressed probe before activation. Upgrade backs up the state
database, enforces the supported schema window, preserves the active engine
identity, and performs a semantic health check before switching the current
release. A failed check rolls back automatically. It does not update Fedora,
restart unrelated Docker containers, or reboot the host.

After activation, verify both control-plane and workload state:

```bash
sparkplane dgx-spark status --json
sparkplane dgx-spark doctor --json
sparkplane dgx-spark ps --json
```

If the new release cannot become healthy, return to the previous release:

```bash
sparkplane dgx-spark rollback --dry-run --json
sparkplane dgx-spark rollback --yes --json
```

Long operations are durable and inspectable. Use the operation identifier from
JSON output to resume events or request cancellation:

```bash
sparkplane dgx-spark operations --json
sparkplane dgx-spark operations 01K... --follow --json
sparkplane dgx-spark operations cancel 01K... --dry-run
```

Certificate rotation is separate from a software release:
`sparkplane dgx-spark cert rotate --dry-run --json`, followed by `--yes` after
review. Use `--ca` only when replacing the certificate authority.

## Add an engine

Add a complete TOML file under `configs/sparkplane/engines/`. Start from the
generic `vllm.toml` when the runtime contract is the same, or from the more
constrained `vllm-qwen38-mmap.toml` when a model requires special arguments.
The schema must remain `sparkplane.engine/v3`.

An engine definition must make runtime policy explicit:

- stable `id`, `family`, priority, supported architecture, and pinned image
  repository plus digest;
- entrypoint and arguments using only the documented placeholders such as
  `{model_snapshot}`, `{served_model}`, and `{port}`;
- offline/cache environment, network, UID, PID and seccomp policy, shared
  memory, resource limits, and startup deadline;
- engine-native health endpoint and semantic identity probe;
- allowlisted routes and methods;
- matcher rules for format, quantization, capabilities, and model types;
- artifact bindings for every engine-consumed auxiliary role, or an explicit
  ignored role; and
- one or more profiles containing model types, context limits, capabilities,
  runtime arguments, and sampling behavior.

Build and publish the ARM64 image separately from the Rust release. Record its
immutable digest in the TOML and make the digest available through the chosen
registry or local image transport before serving a model. A catalog change
without a corresponding image is not deployable.

Do not add model-name branches to Rust. Matching is declarative, and ambiguous
or tied matches fail closed. If the image and transport are unchanged and only
model-specific arguments differ, add a profile instead of creating another
engine. If format, image, security policy, or transport changes, add a new
engine ID and priority deliberately.

Validate the new entry, package it, sign the resulting manifest, and deploy it
through `upgrade`; do not edit `/etc/sparkplane/engines` on a live host. The
packaging script automatically includes every `*.toml` in that directory.

### Rebuild the Qwen3.8 vLLM image

The Qwen3.8 profile uses local image transport and the optimized `5be663`
recipe. It retains native 262,144-token context, BF16 KV, MTP=3, prefix caching,
and piecewise CUDA graphs. Neither `--enforce-eager` nor global
`CUDA_MODULE_LOADING=EAGER` is part of this profile. Preserve the exact recipe
and GPU-device lifecycle contract below when qualifying a replacement image.

Build on ARM64 Spark from the engine directory with all required assets:

```bash
tar -C configs/sparkplane/engines -cf - \
  vllm-qwen38-mmap.Dockerfile checks/qwen38_optimization.py \
  checks/qwen38_sampling.py runtime/qwen38_sampling.py \
  patches/sha256-4ba17608fbb8908c6dfb2e796b8eb9aa4517cdcf6f261c40e8d9d8583b283e25.py \
  patches/sha256-6e5e0a16b41be5795a2ce0bb0e7470641947495c4179402c0298800cb3bf51e0.py |
  ssh dgx-spark 'docker build --progress=plain \
    -t sparkplane/vllm-qwen38-mmap:qualification \
    -f vllm-qwen38-mmap.Dockerfile -'
ssh dgx-spark 'docker image inspect \
  sparkplane/vllm-qwen38-mmap:qualification --format "{{.Id}} {{.Size}}"'
```

The recipe pins the preview base and each imported asset by digest. It tests
PLE gathers during the build, then checks the draft projection and native
kernel loading as serving UID 65534. It also verifies stable CPU-side ordering
of mixed prefill/decode metadata before copying indices to CUDA. A passing Docker build is necessary but
does not replace GPU inference qualification.

The image also warms every supported top-k/top-p sampler batch before accepting
traffic. Run `/opt/llm/checks/qwen38_sampling.py` inside an isolated GPU-enabled
copy of the image (no model loaded); it checks all 96 shape/filter
cases and rejects any late sampler compilation. Qualification must include
normal sampling, mixed greedy/sampled traffic, and 2/4/8 concurrent requests.
A temperature-zero benchmark alone does not exercise the production sampler.
Record these checks separately from temperature-zero throughput measurements.

The optimized profile combines pooled PLE reads, a 65,536-token draft-only
vocabulary, three-token MTP, and prefix caching. The target vocabulary and
RadixArk checkpoint remain unchanged, as do 262,144-token context and the
12 GiB BF16 KV allocation. Prefix caching depends on **both** the recurrent
state restoration patches and deterministic QSA selection; do not enable it
by copying the launch flag into an unpatched image. Imported license notices
are retained under `/opt/llm/licenses`.

Record the locally usable immutable image digest and size in
`vllm-qwen38-mmap.toml`; with Docker's containerd image store, distinguish the
image/manifest digest used by Docker from an OCI configuration blob digest.
Package and sign the catalog as described above. Check for active requests
before the authorized restart, retain the previous image and signed release,
then use managed `stop`, `upgrade`, and `serve` operations. Never patch the
running container or live `/etc` catalog.

Use `tests/fixtures/spark-benchmark/qwen38-optimization.json` with
`scripts/benchmark-spark-engine.py` for identical before/after prompts. Save
exact model, image and profile fingerprints with every result. Check decode
and first-token latency separately, plus cached-versus-cold output, near-limit
context, concurrent requests, reasoning and tool continuation. If correctness
fails, stop the unqualified instance and retain its native logs before cleanup.
Do not silently disable optimizations or promote a control profile as a fix.
Use `sparkplane` to manage the qualified profile through its instance endpoint.

### Preserve GPU access across lifecycle updates

The Spark executor explicitly records the single compute GPU and CUDA control
nodes in Docker's container configuration with read/write permissions. Keep
these mappings alongside the NVIDIA device request: the legacy hook's implicit
grants alone are lost when Docker updates the restart policy after readiness.
This can surface later as a Triton module-load or cuBLAS failure, not just an
NVML error. No driver, cgroup driver, or host security change is required.

The executor unit tests assert these device mappings alongside the NVIDIA
device request. Hardware qualification must additionally verify CUDA inference
after the readiness restart-policy update. Use a signed finite qualification
operation and require exact-container cleanup before recording a terminal result.

## Add a model

For a release-supported model, add a `[[models]]` entry to
`configs/sparkplane/models.toml` (`sparkplane.models/v2`) with:

- aliases and the exact Hugging Face repository plus 40-character commit;
- primary artifact format, quantization, capabilities, and optional
  `engine_profile`;
- exact primary path, byte count, and SHA-256; and
- every auxiliary artifact with a lowercase role, path, byte count, and
  SHA-256.

Roles such as `projector`, `draft_model`, and `weight_shard` are part of the
engine contract. The selected profile must bind or explicitly ignore each
role. Keep paths and hashes exact; do not use a moving branch or an unchecked
URL for a production catalog entry.

After packaging and deployment, acquire the snapshot on Spark with a pinned
revision:

```bash
sparkplane dgx-spark download owner/model \
  --revision <40-character-commit> \
  --alias model:q4 \
  --json
```

Use `--artifact` to select an exact primary file and repeat `--auxiliary
ROLE=PATH` for engine-bound files. The download operation verifies the
snapshot before it can be admitted. Then serve the alias:

```bash
sparkplane dgx-spark serve model:q4 --name model-q4 --json
```

An uncatalogued download can be useful during development, but a release
catalog entry is the reproducible contract that tells matching, admission,
resource planning, and operators exactly what is supported.

## Change protocol or routes

If an engine already supports the model but needs a new OpenAI-compatible
route, update the route allowlist and its tests in the engine catalog. Keep
transport and authentication policy in the control plane. Changes to protocol
semantics, admission, lifecycle, or the TLS/SSH boundary belong in `src/spark`
and require unit plus daemon-level coverage. Every change still ships as a
new signed ARM64 release; never hot-patch a live binary.

## Publish a qualification bundle

Qualification manifests are release artifacts, not ad-hoc remote commands.
Build and publish the ARM64 runner image first, record its registry digest and
total image bytes, then author an exact manifest such as:

```json
{
  "schema": "sparkplane.qualification-manifest/v1",
  "job_id": "spark-flash-adaptive-decoding",
  "operation": "spark_flash_adaptive_decoding_v1",
  "image": "registry.example/qualification-runner@sha256:<64 lowercase hex digits>",
  "target": {
    "architecture": "aarch64",
    "gpu_model": "NVIDIA GB10",
    "compute_capability": "12.1",
    "dgx_build": "<exact installed build>",
    "driver_version": "<exact installed driver>",
    "toolkit_version": "<exact installed toolkit>",
    "protected_fingerprint": "<exact executor-protected fingerprint>"
  },
  "limits": {
    "memory_bytes": 10737418240,
    "image_bytes": 17179869184,
    "pids": 64,
    "cpu_cores": 4,
    "timeout_seconds": 1800,
    "output_bytes": 16777216
  }
}
```

Sign the exact bytes with the same release key whose public half is installed
at `/opt/sparkplane/current/minisign.pub`. Do not reformat the file after signing.
Before approving execution, use the public dry-run and inspect the returned
manifest hash, image digest, exact target, live memory floor, disk reserve, and
isolation document:

```bash
sparkplane dgx-spark qualify \
  --manifest qualification.json \
  --signature qualification.json.minisig \
  --dry-run --json
```

A qualification requires four release-authority inputs:
the built ARM64 runner image, its immutable registry digest, its measured total
image bytes, and a detached manifest signature from the installed release
authority. The CLI cannot substitute an executable or arguments; adding a new
runner mode requires a reviewed finite operation variant and its fixed executor
mapping. The `spark_flash_qsa_v1` variant invokes the fixed runner with
`qsa-target --json`; it accepts no
checkpoint path, mount, executable, or caller-supplied argument. Its Triton JIT
cache is isolated to a bounded executable `/tmp/triton` tmpfs; the parent
`/tmp` remains bounded and no-exec, as it is for adaptive decoding.
The `spark_flash_moe_v1` variant uses the same bounded JIT isolation and fixes
its arguments to `moe-target --json` at the same entrypoint. Installed-runtime
backend captures and native MoE captures each require an immutable signed
image with its exact inputs and measured resource footprint; neither can
attach checkpoint paths or access a managed engine.

Use `spark_flash_kernel_catalog_v1` for the precompiled catalog loader smoke.
It fixes arguments to `kernel-catalog-target --json` at the same entrypoint,
without a JIT environment or executable cache. Build its exact native artifacts
and verification inputs into the signed image; no caller-selected path or
argument is accepted, and the general temporary mount remains no-exec.

Use `spark_flash_cuda_lifecycle_v1` for bounded CUDA ownership and poisoning
qualification. It fixes arguments to `cuda-lifecycle-target --json` at the same
entrypoint, with the same precompiled no-JIT isolation as catalog qualification.
The signed image contains its finite child-process cases and inputs; submitted
arguments cannot alter cycle counts, resource ceilings, or fault selection.

## Troubleshooting and safety checks

Use `spark_flash_memory_admission_v1` for bounded stable-arena qualification.
It fixes arguments to `memory-admission-target --json` at the same entrypoint,
without JIT or caller-selected paths. The signed image binds its inventory,
checkpoint, configuration, native catalog, and finite request-cycle inputs.
Use `make test` for public CLI, signed fake-daemon, fixed
command, and isolation regressions; these host tests do not attest GPU execution.

Use `spark_flash_graph_catalog_v1` for the bounded public graph-catalog scenario.
It fixes arguments to `graph-catalog-target --json` at the same entrypoint.
The signed image contains the checkpoint, full context grid, verified native
catalog, and optional immutable prior measurement. Baseline and measured runs
are separate signed jobs. Both retain the same admission, exclusive lease,
precompiled isolation, watchdog, cleanup, and output limits.

Use `spark_flash_session_v1` for
checkpoint-backed session ownership and cancellation qualification, fixed to
`session-target --json`. The signed image owns the strict target loader,
checkpoint, native catalog, finite scenario, and bounded raw output. Admission,
exclusive high-memory lease, isolation, watchdog, and cleanup are unchanged;
the runner does not claim full model inference from transaction mechanics.

Use `spark_flash_model_prefix_v1` for the distinct checkpoint-backed model-prefix
diagnostic, fixed to `model-prefix-target --json`. The signed image owns the
finite scenario, inputs and source-bound native artifacts; there is no selectable
command, mode, path or JIT environment. Signature checks, admission, the exclusive
lease, watchdog, protected workloads and acknowledged cleanup are unchanged.
Both the workstation client and installed agent/executor must support this wire
value; older versions reject it instead of falling back to another operation.
Build and deploy the normal signed Spark release before submitting the new job.
Host protocol tests do not attest GPU execution, complete inference or tok/s.

Use JSON output for automation and preserve the operation ID. Exit codes are
stable: `2` is a local or artifact error, `3` is a remote policy/state
rejection, and `4` is unreachable transport, authentication, or pinned TLS
failure. `status`, `doctor`, and `ps` distinguish agent health from engine
health.

Common causes are:

- signature or checksum mismatch: obtain the artifact and public key from the
  same tagged release;
- host fingerprint or CA mismatch: stop and correct the local pin, never
  disable verification;
- image digest unavailable: publish or preload the exact ARM64 image;
- model/profile mismatch: inspect format, quantization, capabilities, model
  type, and artifact roles;
- startup or semantic probe failure: inspect the operation events and engine
  logs, then rollback rather than repeatedly restarting a live host; and
- memory admission failure: account for the configured system and emergency
  reserves before reducing limits;
- qualification disk admission failure: correct the signed `image_bytes` or
  free space without crossing the configured disk reserve; and
- qualification output rejection: reduce the fixed runner's diagnostic volume;
  raw output remains bounded and its compressed IPC representation must fit the
  executor frame budget; and
- `read qualification release authority: Permission denied`: deploy a release
  whose agent and executor confinement profiles grant read-only access to the
  release-scoped `minisign.pub`; do not weaken AppArmor or copy the authority
  outside the signed content-addressed release.

The agent has no Docker socket, no arbitrary image or argument injection, and
no host package-update path. Keep those invariants when extending the
catalogs or the CLI.

## Release checklist

Before tagging:

- catalog boundary tests pass and every engine/model entry validates;
- ARM64 build, dependency audit, tests, clippy, and formatting pass;
- docs lint and production site build pass;
- engine image digests and model revisions are recorded and available; and
- the release notes identify the commit and catalog changes.

After tagging:

- verify the signed `SHA256SUMS` and each listed file;
- run `install` or `upgrade --dry-run --json`, then approve the operation;
- check `status`, `doctor`, and `ps`, and run one representative model request;
- retain the operation ID and release manifest; and
- keep the previous release available until semantic health is confirmed.

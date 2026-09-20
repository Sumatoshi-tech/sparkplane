<!-- Template source: Good Docs Project reference template (CC-BY 4.0) — https://www.thegooddocsproject.dev/template/reference. Diátaxis quadrant: reference. -->

# Spark reference

`sparkplane <host>` drives a Spark appliance from your laptop. Use this
page for admission numbers, gateway paths, and engine-policy rules. For
install steps see [How to install the Spark agent](../how-to/install-spark.md).
For serving a model see [How to serve a model on Spark](../how-to/serve-a-model-on-spark.md).

The laptop CLI never holds Docker authority. Engines run on an
internal managed bridge; `ps` reports only active model processes
without printing that address.

Maintenance uses SSH only. `upgrade` accepts the install artifact environment
variables; `rollback` and `cert rotate` remain usable when HTTPS is unavailable.
All three require exactly one of `--dry-run`/`SPARKPLANE_DRY_RUN` or
`--yes`/`SPARKPLANE_YES`, and support `--json`/`SPARKPLANE_JSON` plus
`SPARKPLANE_CONFIG_DIR`. Rollback JSON uses `sparkplane.maintenance/v1`; certificate
rotation uses `sparkplane.certificate-rotation/v1`. Exit codes are `0` success,
`2` local usage/artifact failure, `3` remote compatibility or safety rejection,
and `4` SSH/TLS/agent reachability failure.

The authenticated `GET /api/sparkplane/v1/metrics` endpoint returns bounded
Prometheus text without prompt, generated text, credentials, operation IDs,
commits, or client IDs as labels. It requires an admin token. Engine logs remain
bounded, cursored, redacted, and protected by `logs:read`.

## Synopsis

```text
sparkplane <host> install --dry-run --json
sparkplane <host> install --yes --release-manifest <SHA256SUMS> --release-signature <sig> --release-public-key <pub>
sparkplane <host> upgrade --dry-run --json
sparkplane <host> rollback --dry-run --json
sparkplane <host> cert rotate [--ca] --dry-run --json
sparkplane <host> status --json
sparkplane <host> doctor --json
sparkplane <host> qualify --manifest <json> --signature <minisig> (--dry-run | --yes) [--detach] [--json]
sparkplane <host> serve <model> [--name <instance>] [--detach] [--dry-run] [--json]
sparkplane <host> launch <codex|claude|opencode> [--model <model>] [--config] [--restore] [-y] [-- <agent-args>...]
sparkplane <host> ls [--json]
sparkplane <host> ps [--json]
sparkplane <host> logs <instance> [--limit N]
sparkplane <host> stop <instance>
sparkplane <host> download <repo> --revision <sha> --alias <name>
sparkplane <host> client-config <name> --client <codex|claude-code>
```

## Description

`ls` is the everyday inventory of verified models available to run. Its default
columns are `NAME`, `ID`, `SIZE`, and `MODIFIED`. `ps` is the active lifecycle
view: it omits absent and failed historical records and prints `NAME`, `MODEL`,
`ENGINE`, `CONTEXT`, and `STATE`. `ps --json` contains the same active set. Use
`show`, `logs`, and `ls --json` when complete immutable identity, provenance, or
diagnostic state is required.

Before an engine lifecycle is authorised, the agent requires one
fresh executor-owned snapshot and checks aggregate cold-start
memory, live `MemAvailable`, full-memory PSI, swap-in activity,
disk reserve, immutable model provenance, and the single high-memory
start lease. Stop does not take that lease: reducing memory must remain
available while a start or startup reconciliation is active.

The root-owned `/etc/sparkplane/engines/*.toml` catalog is the serving policy. It
owns each engine image and digest, entrypoint, bounded arguments, environment,
mounts, network, UID, resource envelope, health probe, public route allowlist,
sampling defaults, and finite profiles. A signed `models.toml` entry may select
an `engine_profile` explicitly; otherwise selection falls back to the model's
`config.json` `model_type` and then the engine default. Rust owns schema
validation and security invariants only. It does not embed model IDs, image
versions, digests, tuning values, or per-model commands.

Auxiliary artifact roles are open lowercase identifiers declared by the model
catalog. Each engine configuration must either bind a role to an exact confined
file argument or list the role under `ignored_roles`; an unbound role fails
planning. Profiles can therefore add projectors, MTP drafts, adapters, or future
engine inputs without adding model-aware Rust branches.

`serve` accepts only a verified model reference and optional instance name. The
HTTP schema rejects unknown fields, so callers cannot inject an image,
entrypoint, mount, network, or argv. A model supported by the configured vLLM
version can be downloaded and served without rebuilding Sparkplane. Unsupported models
fail at bounded startup/semantic validation; Sparkplane never substitutes another
engine.

After health check and an exact model-identity completion probe,
the agent publishes:

- OpenAI-compatible routes at
  `https://<spark>:9843/openai/<instance>/v1`
- Anthropic Messages routes under
  `/anthropic/<instance>/v1`

The allowlist exposes authenticated models, completions,
protocol-native chat completions, Responses, Messages, and token
count with bounded SSE and client-side tool continuation. Engine-native health,
metrics, tokenizer, debug, admin, and addresses stay private; the separate
control-plane metrics endpoint requires an admin token.

The gateway preserves Ornith's parsed reasoning as a distinct channel. OpenAI
Chat streams `reasoning_content`; Responses streams reasoning summary item
events and includes the same item in non-stream documents; Anthropic streams a
thinking block and accepts Sparkplane's integrity-checked block back in later turns.
The generated coding-client catalog currently disables requests for reasoning
summaries. Gateway protocol support and the coding client's requested display
mode are separate contracts; changing the catalog requires continuation tests.
Anthropic `display: "omitted"` emits the block and signature without thinking
deltas, while `type: "disabled"` disables thinking in the Ornith template.

Missing sampling values come from the selected profile in `engine.toml` and are
applied consistently to OpenAI Chat, Responses, and Anthropic Messages. Explicit
client values are preserved. Runtime workarounds are profile arguments in the
same file. The legacy Qwen 3.5 profile selects eager execution for its GB10 GEMM
compatibility constraint. The optimized Qwen 3.8 Flash Next profile instead
uses PIECEWISE CUDA graphs, MTP and prefix caching; preserve that qualified
configuration across upgrades and require GPU qualification for changes.

Warming or recovering generations return protocol-native `503`
with `Retry-After` and never inherit a stale route.

## Admission reserves

| Reserve | Value |
|---------|-------|
| System reserve | 8 GiB |
| Emergency floor | 8 GiB |
| Disk reserve | 100 GiB |

Missing or stale telemetry fails closed. The root executor samples
pressure independently and can suppress restart before an emergency
victim is stopped; it never selects unlabeled work.

## Signed GPU qualification jobs

`qualify` runs one immutable release-qualified GPU check without changing any
managed inference engine. The manifest and detached Minisign signature are the
only caller-controlled job inputs. The signed manifest selects a finite
`operation` value; it cannot contain an executable, arguments, mounts,
environment overrides, network settings, or Docker labels.

```bash
sparkplane dgx-spark qualify \
  --manifest adaptive-decoding.json \
  --signature adaptive-decoding.json.minisig \
  --dry-run --json

sparkplane dgx-spark qualify \
  --manifest adaptive-decoding.json \
  --signature adaptive-decoding.json.minisig \
  --yes --json
```

The manifest schema is `sparkplane.qualification-manifest/v1`. It binds a
lowercase slug `job_id`, one reviewed finite operation
(`spark_flash_adaptive_decoding_v1`, `spark_flash_qsa_v1`,
`spark_flash_moe_v1`, `spark_flash_kernel_catalog_v1`,
`spark_flash_cuda_lifecycle_v1`, `spark_flash_memory_admission_v1`,
`spark_flash_graph_catalog_v1`, `spark_flash_session_v1`, or
`spark_flash_model_prefix_v1`), one registry
image pinned by `@sha256:`, the exact host identity,
and bounds for memory, image disk growth, PIDs, CPU, deadline, and raw output.
Both the unprivileged agent and root executor verify the exact manifest bytes
with `/opt/sparkplane/current/minisign.pub`; the executor also rechecks the host
identity and a fresh resource snapshot immediately before create.

The one-shot container has GPU device `0`, no network, a read-only root,
non-root UID/GID 65534, a bounded no-exec `/tmp`, no mounts, no capabilities,
no new privileges, fixed NVIDIA compute/utility capabilities, and restart
policy `no`. The QSA and MoE operations receive a nested 256 MiB executable
`/tmp/triton` cache and `TRITON_CACHE_DIR=/tmp/triton`; its general `/tmp`
remains a separate 64 MiB no-exec mount. Qualification-only labels cannot match
managed-service labels, so
engine reconciliation and emergency handling never adopt the job. One
qualification may run per host, and the normal high-memory transition lease
prevents overlap with engine starts.

Kernel catalog qualification invokes the fixed runner with
`kernel-catalog-target --json`.
Its kernels are precompiled: it receives no JIT environment or executable
cache, and its only temporary mount is the bounded no-exec `/tmp`.

CUDA lifecycle qualification fixes the runner arguments to
`cuda-lifecycle-target --json`, with no JIT
environment or executable cache. Its signed image supplies bounded lifecycle
cases and a parent collector that observes child exit after cleanup. Sparkplane retains
the same signed admission, exclusive lease, output ceilings, and cleanup policy;
it does not interpret a child's claimed receipt as proof of process termination.

Memory admission qualification fixes the runner arguments to
`memory-admission-target --json`. Its signed
image supplies the finite checkpoint, plans, and captured-memory cycles. It
inherits precompiled no-JIT isolation and unchanged signed admission, exclusive
high-memory lease, resource/output limits, and exact-container cleanup.
Memory mechanics do not constitute model inference or throughput evidence.

The durable operation result records the manifest/image identities, refreshed
admission evidence, exit outcome, and lossless stdout/stderr as bounded
`zstd+base64` fields with raw byte counts and SHA-256 identities. A non-zero
exit is a failed operation with its result preserved. Cancel with
`sparkplane <host> operations cancel <operation-id>`; terminal cancellation is
committed only after exact-container cleanup is acknowledged.

## Engine profiles and modalities

- Ornith accepts bounded inline JPEG, PNG, or WebP through OpenAI
  Responses and Anthropic Messages. The adapters validate media
  type, file magic, decoded bytes, image count, and dimensions
  before contacting vLLM. Remote URLs, local paths, traversal,
  unsupported media, and images sent to text-only instances are
  rejected.
- Model types needing finite parser arguments are declared as profiles in
  `engine.toml`. Unknown model types use the declared default profile. Profiles
  cannot override the image, executable, mounts, network, UID, or arbitrary
  command line.
- The `qwen3.8-mtp` llama.cpp profile preserves reasoning and selects
  `draft-mtp` speculation with a verified Q4_0 MTP artifact. It imposes no
  reasoning token guard.

## `client-config`

`--client codex` prints a projection for Codex
`wire_api = "responses"`. `--client claude-code` prints a Claude
Code 2.1.241 projection. Both name the protected token and CA
environment variables without reading, printing, or persisting the
token.

## `launch`

`launch` runs a registered coding-agent executable on the workstation and
routes its model calls to one exact managed Spark instance. A healthy matching
instance is reused; otherwise the configuration-driven `serve` operation is
followed to healthy. The launcher never guesses or downloads a missing model.

`--model` selects a verified model ID, canonical identity, repository, exact
alias, or exact instance name shown by `sparkplane <host> ps`. A stopped selected
instance is re-served under that same managed name. Without `--model`, the saved
host/integration selection is reused; an interactive terminal can select from
installed models. `--config` writes only launch-owned state/config and exits.
`--dry-run` performs no local or remote mutation. `--json` is valid with either
of those non-agent modes. `--restore` removes only Sparkplane-owned Codex
profile/catalog files. `-y` permits the fixed Claude or OpenCode installer when
the executable is absent. Only arguments after `--` are forwarded, without a
shell.

State is serialized with a local lock. Metadata stores only the token ID; the
bearer is held separately in a mode-0600 credential file. The child receives an
inference-only token and pinned CA, never the administrator credential. Claude
uses the native Anthropic route, Codex uses a Sparkplane-owned Responses profile and
catalog, and OpenCode uses process-local `OPENCODE_CONFIG_CONTENT`.

## Examples

```bash
sparkplane dgx-spark serve ornith-1.5:9b --dry-run --json
sparkplane dgx-spark launch codex --model ornith-1.5:9b
sparkplane dgx-spark launch claude --model ornith-1.5:9b -- --permission-mode plan
sparkplane dgx-spark launch opencode --model ornith-1.5:9b
sparkplane dgx-spark serve ornith-1.5:9b --name ornith
sparkplane dgx-spark ps --json
sparkplane dgx-spark logs ornith --limit 100
sparkplane dgx-spark stop ornith
```

## See also

- [How to install the Spark agent](../how-to/install-spark.md)
- [Develop and release the Spark plane](../how-to/develop-spark.md)
- [How to serve a model on Spark](../how-to/serve-a-model-on-spark.md)
- [CLI: `sparkplane`](cli.md#sparkplane)

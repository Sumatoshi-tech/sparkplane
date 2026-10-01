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
sparkplane webui [--host <alias>] [--json]
sparkplane webauthsvc start [--host <alias>] [--json]
sparkplane <host> install --dry-run --json
sparkplane <host> install --yes --release-manifest <SHA256SUMS> --release-signature <sig> --release-public-key <pub>
sparkplane <host> upgrade --dry-run --json
sparkplane <host> rollback --dry-run --json
sparkplane <host> cert rotate [--ca] --dry-run --json
sparkplane <host> status --json
sparkplane <host> doctor --json
sparkplane <host> qualify --manifest <json> --signature <minisig> (--dry-run | --yes) [--detach] [--json]
sparkplane <host> serve <model> [--name <instance>] [--detach] [--dry-run] [--json]
sparkplane <host> launch <codex|claude|opencode> [--mode auto|inherit] [--eco-mode max|none] [--model <model>] [--allow-network] [--config] [--restore] [-y] [-- <agent-args>...]
sparkplane <host> economics [--session <id>] [--json]
sparkplane <host> ls [--json]
sparkplane <host> ps [--json]
sparkplane <host> logs <instance> [--limit N]
sparkplane <host> stop <instance>
sparkplane <host> download <repo> --revision <sha> --alias <name>
sparkplane <host> client-config <name> --client <codex|claude-code>
```

## Description

### Web panel

The panel is served at `https://<spark>:9843/panel/`, using the existing TLS and
allowed-client CIDR policy. Its pages cover overview, analytics, verified models,
managed instances, operations, host health and administration. Access and controls
inherit the selected CLI credential's scopes; `analytics:read` permits usage and
health data. Administrator credentials can manage tokens, audit history,
certificates and release-signed qualification jobs. Updates, rollback and
certificate rotation remain CLI-assisted.

Run `sparkplane webui` to start a temporary loopback authentication service and
open the browser. Click **Authenticate**. Spark creates an HttpOnly browser
session through a single-use, browser-bound handoff; the stored API token is
never sent to browser JavaScript. The CLI exits and closes all local connections
after Spark confirms login. Sessions expire after eight hours or 30 idle minutes;
logout, token expiry and token revocation invalidate access.

For a panel opened directly, run `sparkplane webauthsvc start` on the same
computer as the browser. The page checks `127.0.0.1:9844` and displays Authenticate
when the service appears. Some browsers require local-network permission. If
detection is blocked, use **I started the local service**, then **Authenticate**.
Both entrypoints use the single configured host automatically; use `--host`
when multiple profiles exist. The helper expires after ten minutes; Ctrl-C stops
it earlier. An occupied port produces an error instead of replacing a process.
No sudo or password is required. The browser must trust the installed Spark TLS
certificate; the helper continues to verify the CLI's existing pinned CA.

The panel is enabled by default. Set `[webui] enabled = false` in the agent
configuration to disable it. An optional `origin` restricts browser access to
one exact HTTPS origin; otherwise certificate DNS/IP identities are accepted.
Browser sessions cannot authorize inference endpoints. State-changing panel
requests require the session's CSRF proof and matching origin.

Panel operation lists use `GET /operations?view=summary` for bounded metadata
without qualification result payloads. `GET /operations/<id>` retrieves the full
result on demand; the existing unfiltered operation-list API retains its format.

Analytics count reported inference tokens from ordinary API clients as well as
launched sessions, including embeddings. Unknown and interrupted accounting stays
explicit. Cloud-equivalent cost matches each actual model to the bundled
public-price catalog's dated standard uncached tariff. Unpriced models make the
total unavailable; filter to a priced model to inspect its estimate. Prices for
GPT, Claude or Gemini are never substituted for a different local model.
RTK compression estimates use authenticated,
client-reported byte aggregates and are shown separately. They are not net
ownership savings. No prompts, generated text, images or command output are
stored in analytics.

Request and session details are retained for 90 days, daily usage and RTK rollups
for one year. Health samples are retained at five-second resolution for 24 hours,
minute resolution for 30 days and hourly resolution for one year. History before
this upgrade covers only retained launch sessions. GB10 dedicated VRAM figures
may be unavailable; the panel displays host unified memory separately.
GPU utilization, temperature and power use a fixed, time-limited query inside
an existing exact managed engine. They are unavailable when no such engine or
driver utility is present. The executor keeps its private devices and network.

Panel tables use a separate verified migration ledger after a verified backup.
The qualified engine state schema stays compatible with the preceding release.
The new analytics permission is stored separately from legacy token scopes, so
older releases can decode credentials without granting new permissions.

Additional control API endpoints include `web-auth/approve`, `web-auth/flows/<id>`,
`web-session`, `analytics`, `analytics/requests`, `analytics/sessions`, `audit`,
`health`, `health/history`, `instances/<id>/recover` and
`launch-sessions/<id>/compression`. Analytics use `days` (1–365), `limit` (1–200),
`offset`, and optional `model`, `instance`, `token`, `integration`, `session`
filters; `previous=true` selects the preceding equal period. Days use UTC calendar
boundaries. Paginated results expose `items` and nullable `next_offset`.

Recovery previews and then durably sequences stop and serve, retaining the
qualified engine settings. It rejects catalog/settings differences before
stopping. Every lifecycle action still passes the normal typed executor,
resource admission, confinement and exact-container cleanup checks.
If the agent restarts mid-recovery, the sequence is marked interrupted and its
child lifecycle operations reconcile independently. Inspect those operations
before retrying. In-flight usage from the previous agent is marked interrupted
with unknown counts; it never remains pending indefinitely.

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
  Responses and Anthropic Messages. The gateway resizes and re-encodes oversized
  images to the selected qualified engine profile, preserving aspect ratio and
  transparency. Images already within the profile's limits keep their original
  bytes. The adapters validate media type, file magic, decoded bytes, image count, and dimensions
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
of those non-agent modes. Generated files have a private digest-based ownership
receipt. Launch and restore verify both files before changing either. Codex may
write its own settings, such as `approvals_reviewer`, into the profile; those
extra lines do not block the next launch, and the profile is rewritten from the
managed route. A change to a managed line, a symlink, or an unowned destination
is a conflict. The error prints the path; move that file aside before retrying.
There is no `--force`.
Interrupted publication can be retried safely. `--restore` removes only Sparkplane-owned Codex
profile/catalog files. `-y` permits the fixed Claude or OpenCode installer when
the executable is absent. Only arguments after `--` are forwarded, without a
shell.

Launch also registers its provider in the user-level Codex `config.toml`,
preserving defaults, preferences and other providers. Provider definitions remain
across model changes and `--restore` so saved threads can still resolve their
provider IDs. User edits to a managed provider are preserved and reported as a
conflict. Definitions contain an environment variable name, never a token.
Resume through the launcher to initialize credentials, the pinned CA and image
history handling:

```sh
sparkplane <host> launch codex -- resume [session-id]
```

State is serialized with a local lock. Metadata stores only the token ID; the
bearer is held separately in a mode-0600 credential file. The child receives an
inference-only token and pinned CA, never the administrator credential. Claude
uses the native Anthropic route, Codex uses a Sparkplane-owned Responses profile and
catalog, and OpenCode uses process-local `OPENCODE_CONFIG_CONTENT`.

Codex image attachments follow `input_modalities` reported by the healthy
instance's published gateway route. Image support requires the verified model
and engine vision policy; a vision-capable checkpoint on a text-only route
still accepts only text. Older agents that omit this field default to text.
Relaunch Codex after changing the qualified instance so its catalog is refreshed.
Existing model records retain their verified artifact traits across upgrades.
When a catalog adds vision support, refresh the record through the managed
`download --update-alias` operation and re-serve it with the updated engine
policy before relaunching Codex.
### Image history and upload limits

The Qwen3.8 vLLM policy accepts up to 16 inline PNG, JPEG, or WebP images per request,
sharing at most 512 KiB of image data, with width and height at most 4096 pixels.
The gateway accepts larger source uploads and normalizes them before either
native or translated forwarding; the model's image-count limit still applies.
Each frame receives an equal share of the total image byte budget so earlier
screenshots cannot consume the space needed by subsequent `view_image` results.
Long sessions retain a rolling window of images. At 12 images, older images
already followed by a model response are replaced with explicit text references,
leaving six recent frames. Large source payloads trigger earlier eviction of
processed frames to keep their combined inline payload near 16 MiB; this may
retain fewer than six frames. Frames awaiting their first inspection are
protected. A batch of 16 new images remains supported, and larger new batches
must be split.
Tool calls, call IDs, accompanying text and written observations are retained.
The same policy handles Responses, Chat Completions and nested Anthropic tool
results. Text-only profiles continue rejecting image input.

Vision-enabled `launch` invocations run a private, authenticated loopback adapter.
It streams the historical JSON one item at a time and removes old image payloads
before uploading to the appliance, so repeatedly viewing images does not make
the appliance upload grow with every old screenshot. Source histories may exceed
32 MiB; individual items and the projected request remain capped at 32 MiB, and
history text is capped at 8 MiB. Upload and projection have 120-second deadlines,
with two workers. The adapter uses the pinned appliance CA, refuses redirects,
and exposes only inference routes for the selected instance. Its listener and
invocation credential disappear when the child exits.

Evicted originals are retained privately in
`<config-dir>/spark/image-archive/`, under stable content-derived filenames.
The model receives a path it can reopen with `view_image`; inherited agent
filesystem permissions still apply. The archive uses 0700 directories and 0600
files, deduplicates copies, and caps storage at 512 MiB and 4096 images. Writes
remove the least recently used originals when necessary and expire unused
originals after 90 days. Older references can therefore expire; existing source
files remain another way to inspect those images. Direct gateway clients get
the retention policy, but must prune or compact their own uploads before ingress.

No caption inference is added: descriptions come from the agent's existing
observations. Codex launches compact the conversation at 75% of the qualified
context window, preserving room for new tool results and compaction. Image
retention remains active with `--eco-mode=none`; that switch controls RTK command
compression. These bounds do not change engine context, sampling or resources.
Codex checkpoint requests marked `request_kind=compaction` in
`client_metadata.x-codex-turn-metadata`, without tools, use non-thinking generation
on translated Responses routes. This returns a visible handoff summary instead
of a reasoning-only response that Codex cannot compact. Ordinary coding requests
retain their selected effort; native Responses routes retain their own behavior.
Translation failures count as failed requests while retaining reported tokens,
and streamed errors report the safe failure reason.
Codex `view_image` results and other typed function/custom tool outputs preserve
inline images as multimodal tool content, including resizing and the shared
request image budget. Their base64 is never converted into prompt text. Regular
string and JSON tool outputs retain their text representation.
The root-owned agent configuration has a separate `[gateway]` upload policy:

```toml
[gateway]
request_body_bytes = 33554432 # 32 MiB, including JSON and base64
image_bytes = 16777216       # 16 MiB per decoded source file
image_pixels = 32000000      # all source images combined, before resizing
image_dimension = 16384     # maximum source width or height
max_parallel_requests = 2   # concurrent uploads and image workers
```

These defaults also apply to older configurations without a `[gateway]` section.
Configuration ceilings are 64 MiB requests, 32 MiB source files, 32 million
total source pixels, 16384 pixels per dimension, and four concurrent workers. Uploads have a
30-second deadline; image decoding has a 128 MiB allocation limit. The normalized
inference body and other inference POST bodies are capped at 8 MiB; control API
bodies retain their 1 MiB limit. Saturated upload workers return JSON 503 with
`Retry-After: 1`; oversized uploads return protocol-specific JSON 413. Gateway
rejections do not forward images to the engine. Resizing may reduce fine detail
to fit the qualified profile's byte budget.

Engine request rejections use safe protocol-specific errors: invalid requests
return 400, body limits return 413, and engine capacity limits return 429 with
`Retry-After: 1`. Context overflow returns OpenAI code `context_length_exceeded`
so clients can compact the conversation. Engine error bodies and private request
contents are not forwarded. Unavailable engines and transport failures return
503.

See the [live capability activation procedure](../how-to/develop-spark.md#activate-a-capability-change-on-an-existing-instance)
for signing, cached-model refresh, managed restart, and attachment verification.

### Eco mode and session economics

`--eco-mode=max` is the default. Sparkplane embeds an Apache-2.0 RTK library,
pinned to upstream revision `6d4b77eadee1c66dc1f68466ad77e96d1b6e4989`.
There is no RTK executable download or installer. Invocation-local Codex and
Claude hooks, and an OpenCode plugin, compact supported shell output. The
initial profiles cover Git status, Cargo build/check/clippy/test, pytest and
RTK's bundled declarative filters. Custom RTK configuration is not loaded.
Unknown commands, shell scripts, pipes, redirects, explicit structured output,
binary output and unsuccessful commands pass through unchanged. Captures over
8 MiB per stream pass through; stdout, stderr and the command exit code remain
separate. For an explicit raw escape, use `sparkplane __eco raw -- <command>`.

`--eco-mode=none` disables Sparkplane's compression hooks while retaining
inference accounting. Compression stays inactive with `--mode inherit`, so
rewriting cannot circumvent the agent's own command permissions. Agent hook
support and trust still apply: for Codex, review the Sparkplane hooks in
`/hooks`. Sparkplane does not grant hook trust. Disabled, unsupported or
untrusted hooks leave the session usable and are reported as inactive.
Persistent agent hook settings are not modified.

On a supporting appliance, every launch gets a unique inference-only
credential, valid for at most seven days. It is passed only in the child's
environment and revoked when the coding agent exits. Usage comes from engine
responses before protocol translation, including streamed and native Responses
usage; repeated final usage events count once. Other clients' credentials do
not contribute. The session API is administrator-only. Missing engine usage,
interrupted streams or unavailable accounting produce an explicit incomplete
or unavailable result. Older appliances can still launch agents with usage
shown as unavailable.

The report appears on stderr after normal exit, failure or a handled interrupt.
Finalization has a five-second client timeout. The managed model keeps running;
`stop <instance>` continues to stop only that model. A session report includes
input/output tokens, duration, compressed command counts, estimated output
reduction and two separate USD columns:

- **Cloud equivalent:** known local inference tokens valued at published
  standard uncached input/output rates for the same model in the cloud.
  The catalog currently covers Lyceum's Qwen3.8 Flash Next and Qwen3.8 27B.
  Exact, explicitly listed checkpoint aliases match their cloud model;
  other models show unavailable until a verified tariff is added.
- **Estimated RTK savings:** removed tool-output bytes divided by four,
  valued once at standard input rates. This estimate does not assume repeated
  context reuse and is unavailable when compression metrics are incomplete.

These columns are not added together. Cloud quantization, tokenizers, prompt caching,
hardware and electricity can change actual costs; these are comparisons, not
bills or measured cash savings. The bundled price catalog records official
source links, verification dates, rates and a version. Launch never fetches
prices from the network; each report retains the catalog it used.
Older retained reports keep their original tariff evidence, but unrelated-model
comparisons are no longer displayed.

`sparkplane <host> economics` displays the most recent report;
`economics --session <id> --json` emits the structured report on stdout.
Reports are scoped by host under the Sparkplane config directory, with
mode-0700 directories and mode-0600 files. They contain only aggregate usage
and comparison data, never credentials, prompts, command text or output.
Reports older than 90 days are pruned on save or read; the appliance prunes
session/request ledgers when creating a session. Invocation-local compression
metrics are removed when the launcher exits.

### Internet access for development

Launches default to `--mode auto` (`SPARKPLANE_LAUNCH_MODE=auto`): full
filesystem and network access without action approval prompts. This applies
to the launched local agent session. It is not saved to agent configuration.

| Agent | Auto mode |
| --- | --- |
| Codex | `approval_policy="never"`, `sandbox_mode="danger-full-access"` |
| Claude Code | `bypassPermissions`, sandbox disabled, bypass onboarding prompt skipped |
| OpenCode | All tool permissions allowed in the generated session configuration |

Use `--mode inherit` to keep the agent's own permissions. Forwarded sandbox,
approval, or Claude settings options require inherit mode to avoid conflicting
policies. Managed agent restrictions still apply. Sparkplane's auto mode is
distinct from an agent's similarly named automatic-review mode.
`--dry-run --json` includes `mode`; `--config` and `--restore` never launch an
agent or change its permission settings.

In inherit mode, use `--allow-network` (or `SPARKPLANE_LAUNCH_ALLOW_NETWORK=true`) when the
agent needs to download dependencies, fetch documentation or use remote APIs:

```bash
sparkplane dgx-spark launch codex --mode inherit --allow-network
sparkplane dgx-spark launch claude --mode inherit --allow-network
```

The grant applies only to this invocation. It is not saved in launch state or
agent configuration, and cannot be combined with `--config` or `--restore`.
`--dry-run --json` reports the requested `allow_network` value without changing
anything. In inherit mode without the flag, the agent's existing permissions remain unchanged;
Sparkplane does not impose its own workstation sandbox.

| Agent | Effect of `--allow-network` |
| --- | --- |
| Codex | Sets `sandbox_workspace_write.network_access=true`. Filesystem sandbox mode and approval policy are unchanged. |
| Claude Code | Passes session-only `sandbox.network.allowedDomains=["*"]` through `--settings`. Filesystem restrictions and explicit domain denials remain in force. |
| OpenCode | No extra permission override: OpenCode already has direct network access. Existing shell, web-fetch and other tool approval rules remain in force. |

For Codex, the network setting applies to `workspace-write` sessions. If the
workspace is not trusted or Codex is configured read-only, explicitly select
the development sandbox with `-- --sandbox workspace-write`. Named permission
profiles, managed organization restrictions and explicit forwarded overrides
can impose different policies; this flag does not bypass them. It does not
enable the hosted Responses web-search tool, which the model gateway does not
provide. Commands such as `curl`, package managers and Git can use the network.

Do not combine Claude's forwarded `--settings` with `--allow-network`: the
launcher rejects that ambiguous combination before contacting the appliance.
Use normal Claude settings files for other settings, or omit the flag and pass
your complete session settings yourself.

Codex receives its pinned Spark CA through `CODEX_CA_CERTIFICATE`, not a
launcher-supplied `SSL_CERT_FILE`. This keeps private inference TLS verification
separate from the public/system CA trust used by development subprocesses.
Existing user-supplied certificate and proxy settings are not erased.

Internet access lets subprocesses transmit data they can read; opt in only for
trusted work. This option changes neither the inference-token scope nor the
appliance's private engine networking. Agent controls are documented in the
[Codex sandbox configuration](https://developers.openai.com/codex/security),
[Claude sandbox guide](https://code.claude.com/docs/en/sandboxing), and
[OpenCode permissions reference](https://opencode.ai/docs/permissions/).

## Examples

```bash
sparkplane dgx-spark serve ornith-1.5:9b --dry-run --json
sparkplane dgx-spark launch codex --model ornith-1.5:9b
sparkplane dgx-spark launch claude --mode inherit --model ornith-1.5:9b -- --permission-mode plan
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

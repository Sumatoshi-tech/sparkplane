<!-- Template source: Good Docs Project how-to template (CC-BY 4.0) — https://www.thegooddocsproject.dev/template/how-to. Diátaxis quadrant: how-to. -->

# How to serve a model on Spark

## Goal

Ask a Spark host to run a verified model with its configured engine, wait until the instance is
healthy, and confirm the OpenAI-compatible gateway path answers.

## Prerequisites

- A DGX Spark on the network with the `sparkplane` agent and executor
  already installed and authenticated. If they are not, follow
  [How to install the Spark agent](install-spark.md) first.
- `sparkplane` on your laptop with client credentials configured for
  that host. Substitute `dgx-spark` below for the host name you use.
- Enough free memory and disk on the Spark for the configured engine envelope. Missing
  or stale telemetry fails closed; the agent will refuse the serve
  rather than overcommit.

## Steps

1. Preview admission without starting anything. The report includes
   the 8 GiB system reserve, 8 GiB emergency floor, and 100 GiB disk
   reserve:

   ```bash
   sparkplane dgx-spark serve ornith-1.5:9b --dry-run --json
   ```

   If the dry-run is denied, free memory or disk on the Spark and
   retry. Missing telemetry fails closed: the agent refuses
   rather than guessing.

2. Start the verified model. The agent selects a matching engine from
   `/etc/sparkplane/engines/`; there is no image, recipe, argv, or unsafe-override
   flag on the request:

   ```bash
   sparkplane dgx-spark serve ornith-1.5:9b --name ornith
   ```

3. Watch desired versus observed state. `ps` does not print the
   internal bridge address:

   ```bash
   sparkplane dgx-spark ps --json
   ```

4. To add another model, download an immutable revision and serve its alias:

   ```bash
   sparkplane dgx-spark download owner/model --revision <commit> --alias model:tag
   sparkplane dgx-spark serve model:tag --name model
   ```

   Models supported by the installed vLLM version need no code change. If a
   model requires a finite parser or capability, add a `model_type` profile to
   the engine catalog and deploy it through a signed upgrade. Never
   place image names, model IDs, or launch arguments in Rust.

5. Print a client config for Codex or Claude Code. The output names
   the token and CA environment variables; it does not print or
   persist the token:

   ```bash
   sparkplane dgx-spark client-config ornith --client codex
   sparkplane dgx-spark client-config ornith --client claude-code
   ```

6. Launch a local coding agent against the healthy model. Arguments after `--`
   go directly to that client as exact argv tokens:

   ```bash
   sparkplane dgx-spark launch codex --model ornith-1.5:9b
   sparkplane dgx-spark launch claude --mode inherit --model ornith-1.5:9b -- --permission-mode plan
   sparkplane dgx-spark launch opencode --model ornith-1.5:9b
   ```

   Use `--dry-run --json` to inspect model/instance selection without changing
   local or remote state, or `--config --json` to provision configuration and
   the inference-only token without starting the agent. A missing model is not
   downloaded implicitly; follow the printed immutable `download --revision`
   remediation. `launch` does not edit the primary Claude/OpenCode config or
   the main Codex config.

   Launches default to full access without approvals (`--mode auto`). To retain
   agent sandbox settings while allowing internet access, use inherit mode:

   ```bash
   sparkplane dgx-spark launch codex --mode inherit --allow-network -- --sandbox workspace-write
   sparkplane dgx-spark launch claude --mode inherit --allow-network
   ```

   This is a per-launch opt-in, also available as
   `SPARKPLANE_LAUNCH_ALLOW_NETWORK=true`; it does not disable filesystem
   sandboxing or change approval policy. OpenCode already has network access,
   so its existing tool permissions are left alone. See
   [network controls and limitations](../reference/spark.md#internet-access-for-development).

   After a host reboot, launch waits for an existing running-intent instance
   to recover instead of starting another copy of the model. `--dry-run --json`
   reports `action: "wait"` while recovery is in progress. The wait is bounded
   by the maximum engine startup deadline plus 60 seconds. Check `ps --json`
   if recovery fails or is suppressed; explicitly select an instance with
   `--model <instance-name>` when several instances of the model are recovering.

7. Stop when you are done. An already-absent instance is an idempotent success:

   ```bash
   sparkplane dgx-spark stop ornith
   ```

## Result

`ps --json` shows the instance you named. After health check and
model-identity probe pass, OpenAI-compatible routes are at
`https://<spark>:9843/openai/<instance>/v1` and Anthropic Messages
routes are under `/anthropic/<instance>/v1`. Warming or recovering
generations return protocol-native `503` with `Retry-After`.

For translated Responses requests, an engine can occasionally stop after
reasoning without producing an answer or tool call. The gateway makes at most
one assistant-prefill continuation, retaining the request history, tools and
sampling settings. The engine's chat template closes the retained reasoning
block so generation can continue into the answer. Both attempts share the
remaining output budget, and reported usage includes both generations. This
does not reset or compact the coding session. Unsupported continuations and
repeated reasoning-only stops fail explicitly; malformed tool arguments and
length-limited generations are not retried by this recovery path.

For Ornith, reasoning streams independently from final answer text as OpenAI
`reasoning_content`, Responses reasoning-summary events, or Anthropic thinking
blocks. A recipe with an explicit, qualified reasoning declaration advertises that
capability through the API and generated Codex catalog. Requests that
omit sampling settings receive Ornith's precise-coding defaults; explicit
client settings remain unchanged.

## Default Qwen3.8 Flash Next: Red Hat AI NVFP4

`qwen3.8:flash-next` and `qwen3.8:flash-next-nvfp4` now resolve to
`RedHatAI/Qwen3.8-Flash-Next-NVFP4`, pinned to revision
`c8f2fb1b9869f686b214782036123b10ff96d14a`. The explicit
`qwen3.8:flash-next-redhat` alias selects the same checkpoint. The former default
remains available as `qwen3.8:flash-next-radixark`; its qualified engine is
unchanged. The abliterated alias is separate.

Red Hat's routed experts use NVFP4 with dynamic FP4 activations in the
`compressed-tensors` format. The embedding tables, attention, shared experts,
vision tower and MTP remain BF16. All six weight files, including the separate
MTP checkpoint, are pinned by size and SHA-256. The approximately 186 GB snapshot
requires more disk space than FP4-Local.

The dedicated profile uses the pinned B12X runtime built by
`vllm-qwen38-redhat.Dockerfile`, with bounded `pread` transports for checkpoint
loading and disk-backed BF16 PLE lookup. It retains default seccomp, AppArmor,
private IPC/network, dropped capabilities, a non-root UID and read-only model
mounts. It preserves 262,144-token context, eight sequence slots, FP8 KV with an
explicit 6 GiB allocation, MTP4, prefix caching, reasoning, tool calling and
vision. It does not enable the FP4-Local recipe's optional LM-head quantization
or forced A16 expert mode. Compiler concurrency is bounded within the existing
114 GB resource envelope and 30-minute startup deadline.
The hash-guarded compatibility patch normalizes Red Hat's serialized
`qwen_sparse_attention` label to this runtime's `full_attention` label. The
unchanged indexer geometry still selects Qwen sparse attention; no checkpoint
weights or attention algorithm are changed. Image-build tests use the real
vLLM configuration classes and retain rejection of invalid layer labels and QSA
geometry.

```bash
# --update-alias explicitly moves an already-installed former default.
sparkplane dgx-spark download qwen3.8:flash-next --update-alias --dry-run --json
sparkplane dgx-spark download qwen3.8:flash-next --update-alias --detach --json
sparkplane dgx-spark serve qwen3.8:flash-next --name qwen38-redhat --dry-run --json
sparkplane dgx-spark serve qwen3.8:flash-next --name qwen38-redhat --detach --json
```

Wait for the immutable download to finish, confirm the current instance has no
active requests, and use managed `stop` before serving another large model.
Retain a tested model for rollback. A cold startup may spend several minutes
compiling and tuning kernels; use the operation and engine logs to follow it.
Run public HTTPS checks for reasoning, tools, image input, cancellation and
near-limit context before using the new profile for coding sessions.

If a transfer fails, retry the same immutable download. Admission credits only
space already allocated by regular, singly linked partial files in the managed
cache; sparse holes and unsafe paths receive no credit. The 100 GiB disk reserve
and independent verification of every completed file still apply.

The Red Hat recipe declares reasoning levels through `xhigh`, with `xhigh` as
its Codex default. The control API and public model metadata advertise the
qualified generation's capabilities. Launch normally, or resume the same thread:

```bash
sparkplane dgx-spark launch codex --model qwen3.8:flash-next -- resume <session-id>
```

Explicit per-launch effort choices remain available with Codex's
`-c 'model_reasoning_effort="low"'` after `--`.

[Red Hat's checkpoint and evaluation](https://huggingface.co/RedHatAI/Qwen3.8-Flash-Next-NVFP4).

## Qwen3.8 Flash Next FP4-Local Abliterated

The separate alias `qwen3.8:flash-next-abliterated` selects
`DevelopingDad/Qwen3.8-Flash-Next-FP4-Local-Abliterated` at immutable revision
`fe8c47ff55d0692e28dbcad2c53df2f51294be46`. Its 21 weight shards use NVIDIA's
FP4-Local layout with compressed NVFP4 scales. The B12X engine profile pins
an owned image derived from the publisher's pinned ARM64 runtime. The explicit
`qwen3.8:flash-next-radixark` alias retains the RadixArk checkpoint and its
qualified runtime. The default non-abliterated alias selects Red Hat AI.

```bash
sparkplane dgx-spark download qwen3.8:flash-next-abliterated --dry-run --json
sparkplane dgx-spark download qwen3.8:flash-next-abliterated --detach --json
sparkplane dgx-spark serve qwen3.8:flash-next-abliterated --name qwen38-abliterated --dry-run --json
sparkplane dgx-spark serve qwen3.8:flash-next-abliterated --name qwen38-abliterated --detach --json
sparkplane dgx-spark launch codex --model qwen3.8:flash-next-abliterated
```

Wait for the download operation to finish before serving. Run one large model
at a time: save the current instance identity, confirm it has no active requests,
and use managed `stop` before starting the new instance. Retain the previous
model and engine for rollback. Both download and serve enforce the normal disk
and memory reserves.

The profile retains 262,144-token context, FP8 KV, four-token MTP, prefix caching,
thinking and tool calling. It uses the publisher's disk-backed PLE embeddings
and an explicit 6 GiB KV allocation within a 114 GB resource envelope. Bounded
`pread` transports retain Docker's default seccomp policy, AppArmor, a private
network, a non-root UID and read-only model mounts. Executable JIT temporaries are confined to the managed compile cache;
the general temporary mount remains non-executable. Qualify memory headroom,
long-context reasoning, tool continuation, image input and cancellation on the
actual Spark before relying on this profile for coding sessions.

The checkpoint loader also needs a confined transport: its GB10 bounce reader
normally initializes `io_uring` during weight loading. The owned Dockerfile
applies a hash-guarded patch that selects `pread` with
`B12X_CHECKPOINT_PREAD=1`. It retains the original 16 MiB fixed-slot pool, CUDA
copy worker, descriptor validation, source identity checks, strides and BF16
expansion. Interrupted reads retry; short reads and failed copies fail closed
and drain owned slots before cleanup. The build runs eight real-file I/O cases
with CUDA transport simulated, as UID 65534. These checks also run against the
frozen upstream source fixtures in `make test`; hardware inference remains a
separate qualification step.

Allow time for a cold start: the publisher reports about 23 minutes for first-run
compilation and autotuning, and about five minutes with a populated cache. The
managed compile cache persists across starts; monitor the durable operation and
engine logs until semantic health succeeds.

Disk PLE uses `B12X_DISK_BACKEND=pread`: at most 16 workers read aligned 64 KiB
slots, with the upstream row planner, source validation, weights and scale
scatter preserved. Each wave joins all workers before publishing results or
reusing slots. Short reads, I/O errors and worker creation failures poison the
reader and drain outstanding work. Nine real-file cases exercise the capsule
API, repeated and out-of-range IDs, both planes, unaligned sources, concurrent
calls, bounded staging and failure cleanup. They run in the image build as UID
65534 and against the frozen fixtures in `make test`.

Publisher provenance and runtime recipe:
[model repository](https://huggingface.co/DevelopingDad/Qwen3.8-Flash-Next-FP4-Local-Abliterated/tree/fe8c47ff55d0692e28dbcad2c53df2f51294be46).
The model remains subject to its NVIDIA Open Model Agreement and upstream Qwen
Community License; abliteration does not change those terms.

## See also

- [How to install the Spark agent](install-spark.md)
- [Spark reference](../reference/spark.md)
- [CLI: `sparkplane`](../reference/cli.md#sparkplane)

## `sparkplane`

Inspect, install, and drive one configured DGX Spark appliance.
`<HOST>` is passed to OpenSSH as a single argument; there is no
arbitrary-command escape hatch. OpenSSH owns `known_hosts`, agents,
hardware tokens, and password prompts. Credentials are never accepted
as `sparkplane` arguments and are never stored by `sparkplane`.

Source: `src/spark/cli.rs`. Hidden unit entrypoints (`run-agent`,
`run-executor`, `activate`, `inspect`) are not part of the laptop
CLI.

### Synopsis

```text
sparkplane <HOST> <COMMAND>
```

### Subcommands

| Subcommand | Purpose |
|------------|---------|
| `local migrate-client` | Offline, conflict-checked import of legacy Spark client files; requires `--dry-run` or `--yes`. |
| `install` | Inspect the appliance (`--dry-run`) or apply the signed ARM64 install (`--yes`). |
| `upgrade` | Stage and verify a signed side-by-side release, preserve engines, and automatically roll back failed semantic health. |
| `rollback` | SSH-only exact rollback to the verified preceding control-plane release. |
| `status` | Compact authenticated agent/executor health over pinned HTTPS. |
| `doctor` | Authenticated, read-only compatibility and security checks. |
| `qualify` | Plan or run a release-signed, content-addressed, fixed GPU qualification job. |
| `operations` | Inspect, `--follow`, or `cancel` durable operations. |
| `token` | `create` / `list` / `revoke` scoped bearer tokens. Create returns the secret once on stdout. |
| `download` | Acquire and verify one immutable Hugging Face model snapshot. |
| `serve` | Start a verified model with the root-configured engine after fail-closed admission. |
| `launch` | Run Codex, Claude Code, or OpenCode locally against an exact managed Spark model. |
| `ps` | Compact active model-process table. Absent and failed historical instances are omitted from human and JSON output. |
| `logs` | Bounded, redacted logs for one instance. |
| `stop` | Persist stopped intent, drain, and remove one instance. An already-absent instance is an idempotent success. |
| `ls` | Compact table of verified local models available to run; `--json` returns the complete inventory document. |
| `show` | Immutable identity, provenance, aliases, and references for one model. |
| `rm` | Preview or remove only unreferenced native-cache model data. |
| `client-config` | Render a user-level Codex or Claude Code projection. Names the token env var; does not read or write it. |
| `cert status` | Authenticated leaf-certificate identity. |
| `cert rotate` | SSH-only leaf rotation with overlap; `--ca` rotates and atomically re-pins the local CA. |

### Options (`install`)

| Name | Type | Default | Env | Description |
|------|------|---------|-----|-------------|
| `--dry-run` | bool | `false` | `SPARKPLANE_DRY_RUN` | Upload a content-addressed probe, run `sparkplane bootstrap inspect`, verify the hash, remove the probe. No install. |
| `--yes` | bool | `false` | `SPARKPLANE_YES` | Apply the reviewed manifest. Requires `--release-signature` and `--release-public-key`. |
| `--json` | bool | `false` | `SPARKPLANE_JSON` | Emit `sparkplane.install-manifest/v1`. |
| `--probe` | path | `$XDG_DATA_HOME/sparkplane/appliance-release/sparkplane-aarch64` | `SPARKPLANE_PROBE` | ARM64 feature-minimal probe artefact. |
| `--release-manifest` | path | `SHA256SUMS` beside `--probe` | `SPARKPLANE_RELEASE_MANIFEST` | Signed inventory for the binary and separate catalog TOMLs. |
| `--listen-address` | IP | none | `SPARKPLANE_LISTEN_ADDRESS` | Explicit LAN address for the HTTPS listener. |
| `--listen-port` | u16 | `9843` | `SPARKPLANE_LISTEN_PORT` | HTTPS listener port. |
| `--release-signature` | path | none | `SPARKPLANE_RELEASE_SIGNATURE` | Minisign signature for `SHA256SUMS` (required with `--yes`). |
| `--release-public-key` | path | none | `SPARKPLANE_RELEASE_PUBLIC_KEY` | Pinned minisign public key (required with `--yes`). |
| `--config-dir` | path | Spark config root | `SPARKPLANE_CONFIG_DIR` | Local Spark configuration root. |

`upgrade` accepts the same options. `rollback` and `cert rotate` accept
`--dry-run`, `--yes`, `--json`, and `--config-dir`; exactly one of `--dry-run`
and `--yes` is required. `cert rotate --ca` explicitly replaces the local CA.

### Options (`launch`)

```text
sparkplane <HOST> launch <codex|claude|opencode> [OPTIONS] [-- <AGENT_ARGS>...]
```

| Name | Type | Env | Description |
|------|------|-----|-------------|
| `--model` | string | `SPARKPLANE_LAUNCH_MODEL` | Exact installed model identity or alias. |
| `--config` | bool | `SPARKPLANE_LAUNCH_CONFIG` | Configure launch-owned state and exit. |
| `--restore` | bool | `SPARKPLANE_LAUNCH_RESTORE` | Remove only Sparkplane-owned Codex launch files. |
| `-y`, `--yes` | bool | `SPARKPLANE_YES` | Approve a fixed missing-client installer. |
| `--dry-run` | bool | `SPARKPLANE_DRY_RUN` | Resolve/reuse/admit without mutation. |
| `--json` | bool | `SPARKPLANE_JSON` | Emit `sparkplane.launch-plan/v1`; requires `--dry-run` or `--config`. |
| `--config-dir` | path | `SPARKPLANE_CONFIG_DIR` | Protected Spark configuration root. |

Arguments are accepted only after `--` and are passed directly without a
shell. The agent runs in the current directory with inherited terminal I/O and
receives a separate inference-only token. The Spark administrator credential is
never exposed. Exit codes `1..125` from the child are propagated.

### Options (`qualify`)

```text
sparkplane <HOST> qualify --manifest <PATH> --signature <PATH> (--dry-run | --yes) [OPTIONS]
```

| Name | Type | Env | Description |
|------|------|-----|-------------|
| `--manifest` | path | `SPARKPLANE_QUALIFICATION_MANIFEST` | Exact qualification manifest bytes signed by the installed release authority. |
| `--signature` | path | `SPARKPLANE_QUALIFICATION_SIGNATURE` | Detached Minisign signature over the exact manifest bytes. |
| `--dry-run` | bool | `SPARKPLANE_DRY_RUN` | Verify identity, signature, live resources, and isolation without pulling or creating a container. |
| `--yes` | bool | `SPARKPLANE_YES` | Submit the durable one-shot operation. Exactly one of this and `--dry-run` is required. |
| `--detach` | bool | `SPARKPLANE_DETACH` | Return the durable operation without following it. Valid with `--yes`. |
| `--json` | bool | `SPARKPLANE_JSON` | Emit the plan or operation as JSON. |
| `--idempotency-key` | string | `SPARKPLANE_IDEMPOTENCY_KEY` | Stable retry key; generated when omitted. |
| `--config-dir` | path | `SPARKPLANE_CONFIG_DIR` | Protected Spark configuration root. |

The caller cannot provide an executable, argv, image tag, mount, or network.
`benchmarks:write` is the least-privilege scope for this endpoint.

### Token scopes (`token create --scope`)

`models:read`, `models:write`, `instances:read`, `instances:write`,
`inference`, `logs:read`, `operations:read`, `operations:cancel`,
`benchmarks:read`, `benchmarks:write`. Repeat `--scope` for each.
`benchmarks:write` authorizes signed qualification submission; it never grants
an arbitrary command surface.

### Exit codes

- `0` — success.
- `1` — unexpected failure.
- `2` — usage or local configuration.
- `3` — remote policy or state rejection (admission denied, invalid model
  intent, and similar).
- `4` — OpenSSH/SFTP/agent unreachable, TLS identity mismatch, or
  authentication failure.

### Examples

```bash
sparkplane dgx-spark install --dry-run --json
sparkplane dgx-spark install --yes --release-signature sparkplane-aarch64.minisig \
  --release-public-key sparkplane-release.pub
sparkplane dgx-spark upgrade --dry-run --json
sparkplane dgx-spark rollback --dry-run --json
sparkplane dgx-spark cert rotate --dry-run --json
sparkplane dgx-spark status --json
sparkplane dgx-spark doctor --json
sparkplane dgx-spark qualify --manifest qualification.json \
  --signature qualification.json.minisig --dry-run --json
sparkplane dgx-spark serve ornith-1.5:9b --dry-run --json
sparkplane dgx-spark ps --json
sparkplane dgx-spark token create --name reader --scope models:read \
  --scope operations:read --detach --json
sparkplane dgx-spark client-config ornith --client codex
sparkplane dgx-spark launch codex --model ornith-1.5:9b
sparkplane dgx-spark launch claude --mode inherit --model ornith-1.5:9b -- --permission-mode plan
sparkplane dgx-spark launch opencode --model ornith-1.5:9b
```

### See also

- [How to install the Spark agent](../how-to/install-spark.md)
- [How to serve a model on Spark](../how-to/serve-a-model-on-spark.md)
- [Spark reference](spark.md)

---

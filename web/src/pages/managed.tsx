import { useState, type ReactNode } from "react";
import { Area, AreaChart, CartesianGrid, XAxis } from "recharts";
import { toast } from "sonner";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import {
  Action,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Code,
  DataTable,
  Details,
  ErrorNotice,
  Loading,
  Metric,
  PageTitle,
  Paginated,
  StateBadge,
} from "@/components/panel";
import {
  api,
  bytes,
  count,
  number,
  object,
  permits,
  records,
  text,
  useResource,
  type Doc,
  type Session,
} from "@/lib/api";

function Field({
  label,
  value,
  onChange,
  type = "text",
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: string;
  placeholder?: string;
}) {
  const id = label.replaceAll(" ", "-").toLowerCase();
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}
const route = (kind: string, row: Doc) =>
  `${kind}/${encodeURIComponent(text(row.id))}`;
const quote = (value: unknown) => `'${text(value).replaceAll("'", "'\\''")}'`;

function Serve({
  model,
  onDone,
  instanceName,
}: {
  model: string;
  onDone: () => void;
  instanceName?: string;
}) {
  const [name, setName] = useState(instanceName ?? "");
  const body = { model, name: name || null };
  return (
    <Action
      title="Serve"
      description="Preview the configured engine and available resources before starting this model."
      prepare={async () => {
        const preview = await api("admission", {
          method: "POST",
          body: { ...body, dry_run: true },
        });
        if (!preview.admitted)
          throw new Error(`Admission rejected: ${text(preview.problem_codes)}`);
        return preview;
      }}
      execute={(key) => api("instances", { method: "POST", body, key })}
      onDone={onDone}
    >
      <Field
        label="Instance name"
        value={name}
        onChange={setName}
        placeholder="Automatic"
      />
    </Action>
  );
}
function Download({ onDone }: { onDone: () => void }) {
  const [repository, setRepository] = useState(""),
    [revision, setRevision] = useState("main"),
    [alias, setAlias] = useState(""),
    [artifact, setArtifact] = useState(""),
    [auxiliary, setAuxiliary] = useState(""),
    [updateAlias, setUpdateAlias] = useState(false);
  const body = {
    repository,
    revision,
    alias: alias || null,
    artifact: artifact || undefined,
    auxiliary: auxiliary
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const separator = line.indexOf("=");
        return {
          role: separator < 0 ? line : line.slice(0, separator),
          path: separator < 0 ? "" : line.slice(separator + 1),
        };
      }),
    update_alias: updateAlias,
  };
  return (
    <Action
      title="Download model"
      description="Use a signed catalog alias, or select a repository and exact artifact. Spark verifies the managed snapshot."
      prepare={() =>
        api("downloads", { method: "POST", body: { ...body, dry_run: true } })
      }
      execute={(key) => api("downloads", { method: "POST", body, key })}
      onDone={onDone}
    >
      <div className="space-y-4">
        <Field
          label="Catalog alias or repository"
          value={repository}
          onChange={setRepository}
          placeholder="qwen3.8:flash-next or owner/model"
        />
        <Field
          label="Revision"
          value={revision}
          onChange={setRevision}
          placeholder="main or immutable commit"
        />
        <Field
          label="Alias for custom repository"
          value={alias}
          onChange={setAlias}
          placeholder="Optional"
        />
        <Field
          label="Artifact for custom repository"
          value={artifact}
          onChange={setArtifact}
          placeholder="Optional, e.g. model.gguf"
        />
        <div className="space-y-2">
          <Label htmlFor="auxiliary-artifacts">
            Auxiliary artifacts for custom repository
          </Label>
          <textarea
            id="auxiliary-artifacts"
            className="min-h-20 w-full rounded-md border bg-background p-3 text-sm"
            value={auxiliary}
            onChange={(event) => setAuxiliary(event.target.value)}
            placeholder="One ROLE=PATH per line, e.g. projector=vision/mmproj.gguf"
          />
        </div>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            className="accent-primary"
            checked={updateAlias}
            onChange={(event) => setUpdateAlias(event.target.checked)}
          />
          Replace existing alias after verification
        </label>
      </div>
    </Action>
  );
}
export function ModelsPage({ session }: { session: Session }) {
  const resource = useResource("models", 5000);
  return (
    <div className="space-y-6">
      <PageTitle
        title="Models"
        description="Verified local snapshots, their provenance and managed workloads."
        action={
          permits(session, "models:write") && (
            <Download onDone={resource.refresh} />
          )
        }
      />
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {resource.data ? (
        <DataTable
          rows={records(resource.data.models)}
          empty="No verified local models. Download a catalog model to get started."
          columns={[
            {
              key: "canonical",
              label: "Model",
              render: (row) => (
                <div>
                  <div className="font-medium">{text(row.canonical)}</div>
                  <div className="mt-1 text-xs text-muted-foreground">
                    {text(row.repository)}
                  </div>
                </div>
              ),
            },
            {
              key: "unique_bytes",
              label: "Disk",
              render: (row) => bytes(row.unique_bytes),
            },
            {
              key: "verified_at",
              label: "Verified",
              render: (row) =>
                new Date(text(row.verified_at)).toLocaleDateString(),
            },
            {
              key: "active_instances",
              label: "Instances",
              render: (row) =>
                count(
                  Array.isArray(row.active_instances)
                    ? row.active_instances.length
                    : 0,
                ),
            },
          ]}
          actions={(row) => (
            <>
              <Details title="Details" value={row} />
              {permits(session, "instances:write") && (
                <Serve model={text(row.canonical)} onDone={resource.refresh} />
              )}
              {permits(session, "models:write") && (
                <Action
                  title="Remove"
                  description="Remove this unreferenced snapshot after reviewing shared files and reclaimable space."
                  danger
                  prepare={() =>
                    api(route("models", row), {
                      method: "DELETE",
                      body: { dry_run: true },
                    })
                  }
                  execute={(key) =>
                    api(route("models", row), {
                      method: "DELETE",
                      body: { confirmed: true },
                      key,
                    })
                  }
                  onDone={resource.refresh}
                />
              )}
            </>
          )}
        />
      ) : (
        <Loading error={resource.error} />
      )}
    </div>
  );
}
function Connection({ row }: { row: Doc }) {
  const endpoint = `${window.location.origin}/openai/${encodeURIComponent(text(row.name))}/v1`;
  const model = quote(row.model);
  return (
    <Details
      title="Connect"
      value={{
        openai_base_url: endpoint,
        anthropic_base_url: `${window.location.origin}/anthropic/${encodeURIComponent(text(row.name))}/v1`,
        cli_commands: [
          `sparkplane <host> client-config ${quote(row.name)} --client codex`,
          `sparkplane <host> launch codex --model ${model}`,
          `sparkplane <host> launch claude --model ${model}`,
          `sparkplane <host> launch opencode --model ${model}`,
        ],
        authentication:
          "Use a scoped inference token and your pinned Spark CA.",
      }}
    />
  );
}
export function InstancesPage({ session }: { session: Session }) {
  const resource = useResource("instances", 5000);
  return (
    <div className="space-y-6">
      <PageTitle
        title="Instances"
        description="Managed engines, qualified configuration and generation-specific health."
      />
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {resource.data ? (
        <div className="grid gap-5 xl:grid-cols-2">
          {records(resource.data.instances).map((row) => (
            <Card key={text(row.id)}>
              <CardHeader>
                <div className="flex items-center justify-between gap-3">
                  <CardTitle>{text(row.name)}</CardTitle>
                  <StateBadge value={row.observed} />
                </div>
                <CardDescription>
                  {text(row.model)} · {text(row.engine_id)}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-5">
                <div className="grid grid-cols-3 gap-3 text-sm">
                  <div>
                    <p className="text-xs text-muted-foreground">Generation</p>
                    <p className="mt-1 font-mono">{text(row.generation)}</p>
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">Context</p>
                    <p className="mt-1 font-mono">
                      {count(row.context_window)}
                    </p>
                  </div>
                  <div>
                    <p className="text-xs text-muted-foreground">
                      Steady envelope
                    </p>
                    <p className="mt-1 font-mono">
                      {bytes(object(row.resources).steady_peak_bytes)}
                    </p>
                  </div>
                </div>
                {row.last_failure != null && (
                  <p className="rounded-md border border-amber-500/20 p-3 text-xs text-amber-200">
                    {text(row.last_failure)}
                  </p>
                )}
                <div className="flex flex-wrap gap-2">
                  <Connection row={row} />
                  <Details title="Configuration" value={row} />
                  {permits(session, "logs:read") && (
                    <Details
                      title="Logs"
                      path={`${route("instances", row)}/logs?limit=100`}
                    />
                  )}
                  {permits(session, "instances:write") &&
                    (row.desired === "running" ? (
                      <>
                        <Action
                          title="Stop"
                          description="Persist stopped intent, drain requests and remove this exact managed container."
                          danger
                          prepare={() =>
                            api(route("instances", row), {
                              method: "DELETE",
                              body: { dry_run: true },
                            })
                          }
                          execute={(key) =>
                            api(route("instances", row), {
                              method: "DELETE",
                              body: { timeout_seconds: 30 },
                              key,
                            })
                          }
                          onDone={resource.refresh}
                        />
                        <Action
                          title="Recover"
                          description="Stop, then start a managed generation using the qualified settings. Recovery continues on Spark after you close this page."
                          prepare={() =>
                            api(`${route("instances", row)}/recover`, {
                              method: "POST",
                              body: { dry_run: true },
                            })
                          }
                          execute={(key) =>
                            api(`${route("instances", row)}/recover`, {
                              method: "POST",
                              body: { dry_run: false },
                              key,
                            })
                          }
                          onDone={resource.refresh}
                        />
                      </>
                    ) : (
                      <Serve
                        model={text(row.model)}
                        instanceName={text(row.name)}
                        onDone={resource.refresh}
                      />
                    ))}
                </div>
              </CardContent>
            </Card>
          ))}
          {records(resource.data.instances).length === 0 && (
            <Card>
              <CardContent className="py-14 text-center text-sm text-muted-foreground">
                No managed instances. Serve a verified model from Models.
              </CardContent>
            </Card>
          )}
        </div>
      ) : (
        <Loading error={resource.error} />
      )}
    </div>
  );
}
const terminal = (state: unknown) =>
  ["succeeded", "failed", "cancelled"].includes(text(state));
export function OperationsPage({ session }: { session: Session }) {
  const resource = useResource("operations?view=summary", 3000);
  return (
    <div className="space-y-6">
      <PageTitle
        title="Operations"
        description="Live progress and durable history for downloads, lifecycle changes and qualification."
      />
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {resource.data ? (
        <DataTable
          rows={records(resource.data.operations)}
          columns={[
            { key: "kind", label: "Operation" },
            { key: "target", label: "Target" },
            {
              key: "state",
              label: "State",
              render: (row) => <StateBadge value={row.state} />,
            },
            {
              key: "progress",
              label: "Progress",
              render: (row) => {
                const progress = object(row.progress);
                return (
                  <div className="max-w-72">
                    <span className="text-xs">
                      {text(progress.stage)}
                      {progress.total
                        ? ` · ${count(progress.current)} / ${count(progress.total)} ${text(progress.unit)}`
                        : ""}
                    </span>
                    {number(progress.total) > 0 && (
                      <progress
                        className="mt-2 h-1.5 w-full accent-primary"
                        value={number(progress.current)}
                        max={number(progress.total)}
                        aria-label="Operation progress"
                      />
                    )}
                  </div>
                );
              },
            },
            {
              key: "updated_at",
              label: "Updated",
              render: (row) => new Date(text(row.updated_at)).toLocaleString(),
            },
          ]}
          actions={(row) => (
            <>
              <Details title="Details" path={route("operations", row)} />
              {permits(session, "operations:cancel") &&
                !terminal(row.state) &&
                (session.token_id === "bootstrap-admin" ||
                  session.token_id === row.actor_token_id) && (
                  <Action
                    title="Cancel operation"
                    description="Request cancellation of this owned operation. Completed operations remain immutable."
                    danger
                    execute={(key) =>
                      api(route("operations", row), { method: "DELETE", key })
                    }
                    onDone={resource.refresh}
                  />
                )}
            </>
          )}
        />
      ) : (
        <Loading error={resource.error} />
      )}
    </div>
  );
}

function HealthValue({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 border-b py-3 last:border-0">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="text-sm tabular-nums">{value}</span>
    </div>
  );
}
export function HealthPage({ session }: { session: Session }) {
  const resource = useResource("health", 5000);
  const doctor = useResource(
    permits(session, "admin") ? "doctor" : null,
    30000,
  );
  const [days, setDays] = useState("1");
  const history = useResource(`health/history?days=${days}&limit=200`, 30000);
  const sample = resource.data,
    executor = object(sample?.executor),
    gpu = object(executor.gpu),
    resources = object(executor.resources),
    database = object(sample?.database);
  const age = sample
    ? Math.max(0, (Date.now() - number(sample.observed_at_unix_ms)) / 1000)
    : null;
  const series = records(history.data?.items)
    .reverse()
    .map((row) => {
      let sample: Doc = {};
      try {
        sample = object(JSON.parse(text(row.sample_json)));
      } catch {
        /* Older malformed samples are unavailable. */
      }
      return {
        time: new Date(number(row.bucket)).toLocaleString(),
        cpu: sample.cpu_percent,
        gpu: object(object(sample.executor).gpu).utilization_percent,
        memory:
          object(object(sample.executor).resources).mem_available_bytes == null
            ? null
            : number(
                object(object(sample.executor).resources).mem_available_bytes,
              ) /
              1024 ** 3,
      };
    });
  const percentage = (value: unknown) =>
    value == null ? "Unavailable" : `${number(value).toFixed(1)}%`;
  return (
    <div className="space-y-6">
      <PageTitle
        title="Health"
        description="Cached read-only measurements. Resource admission keeps its independent safety checks."
        action={
          <StateBadge
            value={age == null ? "unavailable" : age > 15 ? "stale" : "healthy"}
          />
        }
      />
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {sample ? (
        <>
          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
            <Metric
              title="CPU utilization"
              value={percentage(sample.cpu_percent)}
              hint="Measured from consecutive host samples"
            />
            <Metric
              title="Unified memory available"
              value={bytes(resources.mem_available_bytes)}
              hint={`Of ${bytes(resources.mem_total_bytes)} host memory`}
            />
            <Metric
              title="GPU utilization"
              value={percentage(gpu.utilization_percent)}
              hint={text(gpu.name)}
            />
            <Metric
              title="Disk available"
              value={bytes(resources.disk_available_bytes)}
              hint={`Sample age: ${age?.toFixed(0)} seconds`}
            />
          </div>
          <div className="grid gap-6 lg:grid-cols-2">
            <Card>
              <CardHeader>
                <CardTitle>Host pressure</CardTitle>
              </CardHeader>
              <CardContent>
                <HealthValue
                  label="Memory full PSI (10s)"
                  value={percentage(resources.memory_full_psi_avg10_percent)}
                />
                <HealthValue
                  label="Swap-in pages since sample"
                  value={text(resources.swap_in_pages_delta)}
                />
                <HealthValue
                  label="GPU temperature"
                  value={
                    gpu.temperature_celsius == null
                      ? "Unavailable"
                      : `${text(gpu.temperature_celsius)} °C`
                  }
                />
                <HealthValue
                  label="GPU power"
                  value={
                    gpu.power_watts == null
                      ? "Unavailable"
                      : `${text(gpu.power_watts)} W`
                  }
                />
                <HealthValue
                  label="Dedicated GPU memory"
                  value={
                    gpu.dedicated_memory_total_mib == null
                      ? "Unavailable · unified memory architecture"
                      : `${text(gpu.dedicated_memory_used_mib)} / ${text(gpu.dedicated_memory_total_mib)} MiB`
                  }
                />
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>Control plane</CardTitle>
              </CardHeader>
              <CardContent>
                <HealthValue
                  label="Agent version"
                  value={text(sample.agent_version)}
                />
                <HealthValue
                  label="Executor guard heartbeat"
                  value={
                    <StateBadge
                      value={
                        object(executor.health).guard_heartbeat
                          ? "healthy"
                          : "unavailable"
                      }
                    />
                  }
                />
                <HealthValue
                  label="Executor event heartbeat"
                  value={
                    <StateBadge
                      value={
                        object(executor.health).event_heartbeat
                          ? "healthy"
                          : "unavailable"
                      }
                    />
                  }
                />
                <HealthValue
                  label="Database backup"
                  value={
                    <StateBadge
                      value={database.backup_valid ? "healthy" : "unavailable"}
                    />
                  }
                />
                <HealthValue
                  label="Database journal"
                  value={text(database.journal_mode)}
                />
                <HealthValue
                  label="Database queue capacity"
                  value={text(database.queue_capacity)}
                />
              </CardContent>
            </Card>
          </div>
          <Card>
            <CardHeader>
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <CardTitle>Resource history</CardTitle>
                  <CardDescription>
                    CPU and GPU utilization · up to 200 samples across the
                    selected period
                  </CardDescription>
                </div>
                <select
                  aria-label="Health history range"
                  value={days}
                  onChange={(event) => setDays(event.target.value)}
                  className="rounded-md border bg-background p-2 text-sm"
                >
                  {[
                    ["1", "24 hours"],
                    ["7", "7 days"],
                    ["30", "30 days"],
                    ["365", "One year"],
                  ].map(([value, label]) => (
                    <option key={value} value={value}>
                      {label}
                    </option>
                  ))}
                </select>
              </div>
            </CardHeader>
            <CardContent>
              <ErrorNotice error={history.error} refresh={history.refresh} />
              {series.length ? (
                <ChartContainer
                  className="h-64 w-full min-w-0 aspect-auto"
                  config={{
                    cpu: { label: "CPU %", color: "var(--chart-1)" },
                    gpu: { label: "GPU %", color: "var(--chart-2)" },
                  }}
                >
                  <AreaChart accessibilityLayer data={series}>
                    <CartesianGrid vertical={false} stroke="var(--border)" />
                    <XAxis
                      dataKey="time"
                      tickLine={false}
                      axisLine={false}
                      minTickGap={80}
                    />
                    <ChartTooltip content={<ChartTooltipContent />} />
                    <Area
                      dataKey="cpu"
                      stroke="var(--chart-1)"
                      fill="var(--chart-1)"
                      fillOpacity={0.1}
                      connectNulls={false}
                      isAnimationActive={false}
                    />
                    <Area
                      dataKey="gpu"
                      stroke="var(--chart-2)"
                      fill="var(--chart-2)"
                      fillOpacity={0.08}
                      connectNulls={false}
                      isAnimationActive={false}
                    />
                  </AreaChart>
                </ChartContainer>
              ) : (
                <p className="py-12 text-center text-sm text-muted-foreground">
                  Health history is collecting.
                </p>
              )}
            </CardContent>
          </Card>
          {permits(session, "admin") && (
            <Card>
              <CardHeader>
                <CardTitle>Doctor checks</CardTitle>
                <CardDescription>
                  Read-only compatibility and security findings.
                </CardDescription>
              </CardHeader>
              <CardContent>
                <ErrorNotice error={doctor.error} refresh={doctor.refresh} />
                <DataTable
                  rows={records(doctor.data?.checks)}
                  columns={[
                    { key: "code", label: "Check" },
                    {
                      key: "status",
                      label: "Status",
                      render: (row) => <StateBadge value={row.status} />,
                    },
                    { key: "detail", label: "Finding" },
                  ]}
                />
              </CardContent>
            </Card>
          )}
        </>
      ) : (
        <Loading error={resource.error} />
      )}
    </div>
  );
}

const scopes = [
  "models:read",
  "models:write",
  "instances:read",
  "instances:write",
  "inference",
  "logs:read",
  "operations:read",
  "operations:cancel",
  "benchmarks:read",
  "benchmarks:write",
  "analytics:read",
  "admin",
];
function NewToken({ onCreated }: { onCreated: (value: Doc) => void }) {
  const [name, setName] = useState(""),
    [cidrs, setCidrs] = useState(""),
    [expiry, setExpiry] = useState(""),
    [concurrency, setConcurrency] = useState("1"),
    [selected, setSelected] = useState([
      "models:read",
      "instances:read",
      "operations:read",
      "analytics:read",
    ]);
  return (
    <Action
      title="Create token"
      description="Choose the exact permissions for this credential. Its secret is displayed once."
      execute={async (key) => {
        if (!name.trim() || selected.length === 0)
          throw new Error("Enter a name and select permissions");
        const result = await api("tokens", {
          method: "POST",
          key,
          body: {
            name,
            scopes: selected,
            allowed_cidrs: cidrs
              .split(",")
              .map((value) => value.trim())
              .filter(Boolean),
            expires_at: expiry ? new Date(expiry).toISOString() : null,
            max_concurrent_inference: Number(concurrency),
          },
        });
        onCreated(result);
        return result;
      }}
    >
      <div className="space-y-4">
        <Field
          label="Token name"
          value={name}
          onChange={setName}
          placeholder="Workstation or integration name"
        />
        <div className="grid grid-cols-2 gap-3">
          {scopes.map((scope) => (
            <label key={scope} className="flex items-center gap-2 text-xs">
              <input
                type="checkbox"
                className="accent-primary"
                checked={selected.includes(scope)}
                onChange={(event) =>
                  setSelected(
                    event.target.checked
                      ? [...selected, scope]
                      : selected.filter((value) => value !== scope),
                  )
                }
              />
              {scope}
            </label>
          ))}
        </div>
        <Field
          label="Allowed CIDRs"
          value={cidrs}
          onChange={setCidrs}
          placeholder="Optional, comma separated"
        />
        <Field
          label="Expires at"
          value={expiry}
          onChange={setExpiry}
          type="datetime-local"
        />
        <Field
          label="Maximum concurrent inference"
          value={concurrency}
          onChange={setConcurrency}
          type="number"
        />
      </div>
    </Action>
  );
}
function Qualification() {
  const [manifest, setManifest] = useState(""),
    [signature, setSignature] = useState("");
  const submission = (dry_run: boolean) => ({
    manifest_base64: manifest,
    signature,
    dry_run,
  });
  return (
    <Action
      title="Run qualification"
      description="Submit the exact release-signed manifest and detached signature. The signed job owns its resources and finite operation."
      prepare={() =>
        api("qualifications", { method: "POST", body: submission(true) })
      }
      execute={(key) =>
        api("qualifications", { method: "POST", body: submission(false), key })
      }
    >
      <div className="space-y-4">
        <label className="block space-y-2 text-sm">
          Signed manifest file
          <input
            type="file"
            accept="application/json,.json"
            className="block text-xs"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) {
                if (file.size > 262144) {
                  toast.error("Manifest exceeds 256 KiB");
                  return;
                }
                void file
                  .arrayBuffer()
                  .then((buffer) =>
                    setManifest(
                      btoa(
                        Array.from(new Uint8Array(buffer), (value) =>
                          String.fromCharCode(value),
                        ).join(""),
                      ),
                    ),
                  );
              }
            }}
          />
        </label>
        <label className="block space-y-2 text-sm">
          Detached signature file
          <input
            type="file"
            accept=".minisig,text/plain"
            className="block text-xs"
            onChange={(event) => {
              const file = event.target.files?.[0];
              if (file) {
                if (file.size > 4096) {
                  toast.error("Signature exceeds 4 KiB");
                  return;
                }
                void file.text().then(setSignature);
              }
            }}
          />
        </label>
      </div>
    </Action>
  );
}
export function AdminPage({ session }: { session: Session }) {
  const tokens = useResource("tokens", 10000),
    certificates = useResource("certificates/status", 60000),
    status = useResource("status", 30000);
  const [created, setCreated] = useState<Doc>();
  return (
    <div className="space-y-6">
      <PageTitle
        title="Administration"
        description="Credentials, signed qualification, audit evidence and appliance maintenance."
      />
      <Tabs defaultValue="tokens">
        <TabsList>
          <TabsTrigger value="tokens">Tokens</TabsTrigger>
          <TabsTrigger value="audit">Audit</TabsTrigger>
          <TabsTrigger value="qualification">Qualification</TabsTrigger>
          <TabsTrigger value="appliance">Appliance</TabsTrigger>
        </TabsList>
        <TabsContent value="tokens" className="space-y-4">
          <div className="flex justify-end">
            <NewToken
              onCreated={(value) => {
                setCreated(value);
                tokens.refresh();
              }}
            />
          </div>
          <ErrorNotice error={tokens.error} refresh={tokens.refresh} />
          {tokens.data ? (
            <DataTable
              rows={records(tokens.data.tokens)}
              columns={[
                { key: "name", label: "Name" },
                { key: "scopes", label: "Permissions" },
                { key: "expires_at", label: "Expires" },
                { key: "max_concurrent_inference", label: "Concurrency" },
                {
                  key: "revoked_at",
                  label: "State",
                  render: (row) => (
                    <StateBadge value={row.revoked_at ? "revoked" : "active"} />
                  ),
                },
              ]}
              actions={(row) => (
                <>
                  <Details title="Details" value={row} />
                  {!row.revoked_at && (
                    <Action
                      title="Revoke"
                      description={
                        row.id === session.token_id
                          ? "This revokes the credential backing your current browser session and logs you out."
                          : "Immediately invalidate this credential and all browser sessions backed by it."
                      }
                      danger
                      execute={(key) =>
                        api(route("tokens", row), { method: "DELETE", key })
                      }
                      onDone={tokens.refresh}
                    />
                  )}
                </>
              )}
            />
          ) : (
            <Loading error={tokens.error} />
          )}
        </TabsContent>
        <TabsContent value="audit">
          <Paginated
            path="audit?days=90"
            columns={[
              { key: "occurred_at", label: "Time" },
              { key: "actor_token_id", label: "Actor" },
              { key: "action", label: "Action" },
              { key: "target", label: "Target" },
              { key: "outcome", label: "Outcome" },
            ]}
          />
        </TabsContent>
        <TabsContent value="qualification" className="space-y-5">
          <Card>
            <CardHeader>
              <CardTitle>Signed GPU qualification</CardTitle>
              <CardDescription>
                Preview and run finite jobs authorized by the installed release
                authority.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Qualification />
            </CardContent>
          </Card>
          <p className="text-sm text-muted-foreground">
            Qualification results and signed identities appear in Operations.
            Generation-specific engine settings are preserved across managed
            upgrades.
          </p>
        </TabsContent>
        <TabsContent value="appliance" className="space-y-5">
          <Card>
            <CardHeader>
              <CardTitle>TLS certificate</CardTitle>
              <CardDescription>
                Browser trust uses the installed Spark certificate. CLI
                transport retains its pinned CA.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <ErrorNotice
                error={certificates.error}
                refresh={certificates.refresh}
              />
              <DataTable
                rows={certificates.data ? [certificates.data] : []}
                columns={[
                  { key: "valid", label: "Valid" },
                  { key: "dns_sans", label: "DNS identities" },
                  { key: "ip_sans", label: "IP identities" },
                ]}
              />
            </CardContent>
          </Card>
          <Card>
            <CardHeader>
              <CardTitle>Signed maintenance</CardTitle>
              <CardDescription>
                Run these commands on your configured workstation, replacing
                &lt;host&gt; with its Spark profile.
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-3">
              <Code value="sparkplane <host> upgrade --dry-run --json" />
              <Code value="sparkplane <host> rollback --dry-run --json" />
              <Code value="sparkplane <host> cert rotate --dry-run --json" />
              <Details
                title="Current release and appliance status"
                value={status.data}
              />
            </CardContent>
          </Card>
        </TabsContent>
      </Tabs>
      <Dialog
        open={created !== undefined}
        onOpenChange={(open) => {
          if (!open) setCreated(undefined);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Save your new token</DialogTitle>
            <DialogDescription>
              The secret is displayed once. Keep it in your credential store.
            </DialogDescription>
          </DialogHeader>
          {created?.bearer_token ? (
            <Code value={text(created.bearer_token)} />
          ) : (
            <p className="text-sm text-muted-foreground">
              This operation was already submitted. Its secret is not returned
              again.
            </p>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

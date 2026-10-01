import { useEffect, useState } from "react";
import {
  Activity,
  ArrowUpRight,
  BarChart3,
  Box,
  ChevronDown,
  Cpu,
  LayoutDashboard,
  LogOut,
  Server,
  Settings,
  ShieldCheck,
  Terminal,
  Zap,
} from "lucide-react";
import { Area, AreaChart, CartesianGrid, XAxis } from "recharts";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
  SidebarTrigger,
  useSidebar,
} from "@/components/ui/sidebar";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/sonner";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  api,
  ApiError,
  count,
  exportCsv,
  money,
  number,
  object,
  permits,
  records,
  setCsrf,
  text,
  useResource,
  type Doc,
  type Session,
} from "@/lib/api";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Code,
  DataTable,
  ErrorNotice,
  Loading,
  Metric,
  PageTitle,
  Paginated,
  StateBadge,
} from "@/components/panel";
import {
  AdminPage,
  HealthPage,
  InstancesPage,
  ModelsPage,
  OperationsPage,
} from "@/pages/managed";

const navigation = [
  { id: "overview", title: "Overview", icon: LayoutDashboard, scope: null },
  {
    id: "analytics",
    title: "Analytics",
    icon: BarChart3,
    scope: "analytics:read",
  },
  { id: "models", title: "Models", icon: Box, scope: "models:read" },
  {
    id: "instances",
    title: "Instances",
    icon: Server,
    scope: "instances:read",
  },
  {
    id: "operations",
    title: "Operations",
    icon: Activity,
    scope: "operations:read",
  },
  { id: "health", title: "Health", icon: Cpu, scope: "analytics:read" },
  { id: "admin", title: "Administration", icon: Settings, scope: "admin" },
];
function Logo() {
  return (
    <div className="flex items-center gap-3">
      <div className="flex size-9 items-center justify-center rounded-lg bg-primary text-primary-foreground">
        <Zap className="size-5" />
      </div>
      <span className="text-lg font-semibold tracking-tight">
        Sparkplane<span className="text-primary">.</span>
      </span>
    </div>
  );
}

function Login() {
  const [ready, setReady] = useState(window.location.hash === "#local-auth");
  const [checking, setChecking] = useState(true);
  useEffect(() => {
    let active = true;
    const controller = new AbortController();
    const check = async () => {
      try {
        const response = await fetch("http://127.0.0.1:9844/health", {
          signal: AbortSignal.any([
            controller.signal,
            AbortSignal.timeout(1200),
          ]),
          credentials: "omit",
          cache: "no-store",
        });
        const value = object(await response.json());
        if (
          active &&
          value.protocol === "sparkplane.webauth/v1" &&
          value.origin === window.location.origin
        )
          setReady(true);
      } catch {
        /* A stopped helper and blocked local-network detection both use the redirect fallback. */
      } finally {
        if (active) setChecking(false);
      }
    };
    void check();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void check();
    }, 2000);
    return () => {
      active = false;
      controller.abort();
      clearInterval(timer);
    };
  }, []);
  return (
    <main className="grid-panel flex min-h-svh flex-col">
      <header className="p-8">
        <Logo />
      </header>
      <div className="flex flex-1 items-center justify-center px-5 pb-20">
        <Card className="w-full max-w-lg border-primary/15 shadow-2xl">
          <CardHeader className="gap-4 px-8 pt-8">
            <div className="flex size-12 items-center justify-center rounded-xl border border-primary/25 bg-primary/10">
              <ShieldCheck className="size-6 text-primary" />
            </div>
            <CardTitle className="text-2xl">Your Spark, one place.</CardTitle>
            <CardDescription>
              Models, operations and the economics of your AI workloads.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-5 px-8 pb-8">
            {ready ? (
              <>
                <div className="flex items-center gap-2 text-sm text-chart-2">
                  <span className="status-dot" />
                  Local authentication service ready
                </div>
                <Button className="w-full" size="lg" asChild>
                  <a href="http://127.0.0.1:9844/auth/start">
                    Authenticate
                    <ArrowUpRight />
                  </a>
                </Button>
                <p className="text-xs leading-5 text-muted-foreground">
                  Uses your existing Sparkplane credential. The temporary local
                  service closes after login.
                </p>
              </>
            ) : (
              <>
                <p className="text-sm">
                  Start the local authentication service in a terminal on this
                  computer.
                </p>
                <Code value="sparkplane webauthsvc start" />
                <p className="text-xs leading-5 text-muted-foreground">
                  {checking
                    ? "Checking for your local service…"
                    : "Waiting for the local service. Authenticate appears when it is detected."}{" "}
                  If your browser blocks detection, allow local-network access
                  or use the button below after starting the service.
                </p>
                <Button
                  variant="outline"
                  className="w-full"
                  onClick={() => setReady(true)}
                >
                  I started the local service
                </Button>
              </>
            )}
            <div className="border-t pt-5 text-sm text-muted-foreground">
              <span className="mb-2 flex items-center gap-2">
                <Terminal className="size-4" />
                Open directly from your terminal
              </span>
              <Code value="sparkplane webui" />
            </div>
          </CardContent>
        </Card>
      </div>
      <footer className="p-6 text-center text-xs text-muted-foreground">
        Authenticated access · LAN / VPN · {window.location.host}
      </footer>
    </main>
  );
}

function UsageChart({ series }: { series: Doc[] }) {
  if (series.length === 0)
    return (
      <div className="flex h-64 items-center justify-center text-sm text-muted-foreground">
        Usage will appear after your first model request.
      </div>
    );
  return (
    <ChartContainer
      className="h-64 w-full min-w-0 aspect-auto"
      config={{
        input_tokens: { label: "Input tokens", color: "var(--chart-1)" },
        output_tokens: { label: "Output tokens", color: "var(--chart-2)" },
      }}
    >
      <AreaChart
        accessibilityLayer
        data={series}
        margin={{ left: 6, right: 6, top: 12 }}
      >
        <CartesianGrid vertical={false} stroke="var(--border)" />
        <XAxis
          dataKey="day"
          tickLine={false}
          axisLine={false}
          tickMargin={10}
          tickFormatter={(value) => String(value).slice(5)}
        />
        <ChartTooltip content={<ChartTooltipContent />} />
        <Area
          type="monotone"
          dataKey="input_tokens"
          stroke="var(--chart-1)"
          fill="var(--chart-1)"
          fillOpacity={0.14}
          isAnimationActive={false}
        />
        <Area
          type="monotone"
          dataKey="output_tokens"
          stroke="var(--chart-2)"
          fill="var(--chart-2)"
          fillOpacity={0.08}
          isAnimationActive={false}
        />
      </AreaChart>
    </ChartContainer>
  );
}
function UsageMetrics({ data }: { data: Doc }) {
  const summary = object(data.summary),
    rtk = object(data.rtk);
  const requests = number(summary.requests),
    failed = number(summary.failed_requests),
    pending = number(summary.pending_requests);
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
      <Metric
        title="Tokens processed"
        value={count(
          number(summary.input_tokens) + number(summary.output_tokens),
        )}
        hint={`${count(summary.input_tokens)} input · ${count(summary.output_tokens)} output`}
      />
      <Metric
        title="Cloud-equivalent cost"
        value={money(summary.cloud_equivalent_usd_nanos)}
        hint={
          number(summary.unpriced_requests) > 0
            ? `${count(summary.unpriced_requests)} requests without a verified same-model tariff`
            : "Same models · public standard tariffs"
        }
      />
      <Metric
        title="RTK tokens avoided"
        value={
          number(rtk.covered_sessions) > 0
            ? `≈ ${count(rtk.estimated_saved_tokens)}`
            : "—"
        }
        hint={`${count(rtk.covered_sessions)} sessions with valid client reports`}
      />
      <Metric
        title="Completed successfully"
        value={
          requests - pending > 0
            ? `${(((requests - failed - pending) / (requests - pending)) * 100).toFixed(1)}%`
            : "—"
        }
        hint={`${count(requests)} requests · ${count(pending)} pending`}
      />
    </div>
  );
}
function Overview({ session }: { session: Session }) {
  const usage = useResource(
    permits(session, "analytics:read") ? "analytics?days=7" : null,
  );
  const instances = useResource(
    permits(session, "instances:read") ? "instances" : null,
    5000,
  );
  const operations = useResource(
    permits(session, "operations:read") ? "operations?view=summary" : null,
    5000,
  );
  const status = useResource(
    permits(session, "instances:read") ? "status" : null,
    10000,
  );
  return (
    <div className="space-y-6">
      <PageTitle
        title="Overview"
        description="A clear view of your appliance, workloads and savings."
      />
      <ErrorNotice
        error={status.error ?? usage.error ?? instances.error}
        refresh={() => {
          status.refresh();
          usage.refresh();
          instances.refresh();
        }}
      />
      {usage.data ? (
        <UsageMetrics data={usage.data} />
      ) : permits(session, "analytics:read") ? (
        <Loading error={usage.error} />
      ) : (
        <Card>
          <CardContent className="pt-6 text-sm text-muted-foreground">
            This credential does not include analytics access.
          </CardContent>
        </Card>
      )}
      <div className="grid gap-6 xl:grid-cols-[1.6fr_1fr]">
        {permits(session, "analytics:read") && (
          <Card>
            <CardHeader>
              <CardTitle>Token activity</CardTitle>
              <CardDescription>
                Last seven days · reported input and output tokens
              </CardDescription>
            </CardHeader>
            <CardContent>
              <UsageChart series={records(usage.data?.series)} />
            </CardContent>
          </Card>
        )}
        <Card>
          <CardHeader>
            <CardTitle>Appliance status</CardTitle>
            <CardDescription>
              {permits(session, "instances:read")
                ? window.location.host
                : "Instance status is outside this credential’s permissions."}
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-5">
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Agent</span>
              <StateBadge value={status.data?.agent} />
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">Executor</span>
              <StateBadge value={status.data?.executor} />
            </div>
            <div className="flex items-center justify-between">
              <span className="text-sm text-muted-foreground">
                Healthy instances
              </span>
              <span className="font-mono">
                {instances.data
                  ? records(instances.data.instances).filter(
                      (row) => row.healthy,
                    ).length
                  : "—"}
              </span>
            </div>
            {records(status.data?.degraded_reasons).map((reason, index) => (
              <p
                key={index}
                className="rounded-md border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-200"
              >
                {text(reason.detail)}
              </p>
            ))}
            <p className="border-t pt-4 text-xs leading-5 text-muted-foreground">
              Cloud-equivalent cost estimates what reported usage would cost at
              public tariffs for the same models. RTK estimates are shown
              separately.
            </p>
          </CardContent>
        </Card>
      </div>
      {permits(session, "operations:read") && (
        <Card>
          <CardHeader>
            <CardTitle>Recent operations</CardTitle>
            <CardDescription>
              Progress survives browser reloads and disconnects.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <DataTable
              rows={records(operations.data?.operations).slice(0, 5)}
              columns={[
                { key: "kind", label: "Operation" },
                { key: "target", label: "Target" },
                {
                  key: "state",
                  label: "State",
                  render: (row) => <StateBadge value={row.state} />,
                },
                {
                  key: "updated_at",
                  label: "Updated",
                  render: (row) =>
                    new Date(text(row.updated_at)).toLocaleString(),
                },
              ]}
            />
          </CardContent>
        </Card>
      )}
    </div>
  );
}
function Analytics() {
  const initial = new URLSearchParams(window.location.search);
  const [days, setDays] = useState(initial.get("days") ?? "7");
  const [model, setModel] = useState(initial.get("model") ?? "all");
  const [dimension, setDimension] = useState("model");
  const [filters, setFilters] = useState(() =>
    Object.fromEntries(
      ["instance", "token", "integration", "session"].map((key) => [
        key,
        initial.get(key) ?? "",
      ]),
    ),
  );
  const query = new URLSearchParams({ days });
  if (model !== "all") query.set("model", model);
  for (const [key, value] of Object.entries(filters))
    if (value) query.set(key, value);
  const queryText = query.toString();
  const resource = useResource(`analytics?${queryText}`);
  const previous = useResource(`analytics?${queryText}&previous=true`);
  useEffect(() => {
    const url = new URL(window.location.href);
    url.search = queryText;
    window.history.replaceState(null, "", url);
  }, [queryText]);
  const summary = object(resource.data?.summary),
    rtk = object(resource.data?.rtk);
  const catalog = object(resource.data?.catalog);
  const tariffs = records(summary.comparisons).filter(
    (row) => row.cloud_model != null,
  );
  const breakdown = records(object(resource.data?.breakdown)[dimension]);
  const requestChange = previous.data
    ? number(summary.requests) - number(object(previous.data.summary).requests)
    : null;
  return (
    <div className="space-y-6">
      <PageTitle
        title="Analytics"
        description="Understand usage and the public-cloud value of your local inference."
        action={
          <Button
            variant="outline"
            onClick={() =>
              exportCsv(
                records(resource.data?.series),
                [
                  "day",
                  "requests",
                  "input_tokens",
                  "output_tokens",
                  "unknown_usage_requests",
                  "failed_requests",
                ],
                "sparkplane-usage.csv",
              )
            }
          >
            Export daily usage
          </Button>
        }
      />
      <div className="flex flex-wrap gap-3">
        <Select value={days} onValueChange={setDays}>
          <SelectTrigger aria-label="Date range">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {[
              ["1", "Today"],
              ["7", "Last 7 days"],
              ["30", "Last 30 days"],
              ["90", "Last 90 days"],
              ["365", "Last year"],
            ].map(([value, label]) => (
              <SelectItem key={value} value={value}>
                {label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={model} onValueChange={setModel}>
          <SelectTrigger aria-label="Model filter">
            <SelectValue placeholder="All models" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All models</SelectItem>
            {records(object(resource.data?.breakdown).model).map((row) => (
              <SelectItem key={text(row.model)} value={text(row.model)}>
                {text(row.model)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        {Object.entries(filters).map(([key, value]) => (
          <label key={key} className="space-y-1 text-xs text-muted-foreground">
            {key[0].toUpperCase() + key.slice(1)}
            <input
              className="h-9 w-full rounded-md border bg-background px-3 text-sm text-foreground"
              value={value}
              placeholder={`All ${key}s`}
              onChange={(event) =>
                setFilters({ ...filters, [key]: event.target.value })
              }
            />
          </label>
        ))}
      </div>
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {resource.data ? (
        <>
          <UsageMetrics data={resource.data} />
          <p className="text-xs text-muted-foreground">
            {requestChange == null
              ? "Previous period unavailable"
              : `${requestChange >= 0 ? "+" : ""}${count(requestChange)} requests versus the previous period`}{" "}
            · {count(summary.unknown_usage_requests)} requests with unknown
            usage · {count(summary.accounting_errors)} incomplete accounting
            records
          </p>
          <div className="grid gap-6 xl:grid-cols-[1.7fr_1fr]">
            <Card>
              <CardHeader>
                <CardTitle>Token activity</CardTitle>
                <CardDescription>
                  {text(resource.data.from)} → {text(resource.data.to)} · UTC
                  calendar days
                </CardDescription>
              </CardHeader>
              <CardContent>
                <UsageChart series={records(resource.data.series)} />
              </CardContent>
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>Compression economics</CardTitle>
                <CardDescription>Client-reported RTK estimates</CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <Metric
                  title="Estimated RTK value"
                  value={
                    rtk.estimated_saved_usd_nanos != null
                      ? `≈ ${money(rtk.estimated_saved_usd_nanos)}`
                      : "Unknown"
                  }
                  hint="Input-token equivalent; measured bytes ÷ 4"
                />
                <p className="text-xs leading-6 text-muted-foreground">
                  {count(rtk.commands)} filtered commands.{" "}
                  {count(rtk.covered_sessions)} valid reports out of{" "}
                  {count(rtk.reported_sessions)} reported sessions. This
                  estimate is separate from cloud-equivalent inference cost.
                </p>
                {tariffs.map((tariff) => (
                  <a
                    key={text(tariff.model)}
                    className="block break-all text-xs text-chart-2 underline"
                    href={text(tariff.source)}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {text(tariff.provider)} · {text(tariff.cloud_model)} ·
                    verified {text(tariff.verified_at)}
                  </a>
                ))}
                {number(rtk.unpriced_sessions) > 0 && (
                  <p className="text-xs text-muted-foreground">
                    {count(rtk.unpriced_sessions)} RTK sessions without a
                    verified same-model tariff.
                  </p>
                )}
                <p className="text-xs text-muted-foreground">
                  Catalog {text(catalog.version)} · standard uncached pricing.
                  Same model; cloud quantization may differ. Cache and reasoning
                  subdivisions are unavailable unless reported by the engine.
                </p>
              </CardContent>
            </Card>
          </div>
          <div className="grid gap-4 sm:grid-cols-3">
            <Metric
              title="Mean request duration"
              value={
                summary.mean_duration_ms == null
                  ? "—"
                  : `${(number(summary.mean_duration_ms) / 1000).toFixed(2)}s`
              }
              hint="Finished requests with timing evidence"
            />
            <Metric
              title="Mean time to first output"
              value={
                summary.mean_ttft_ms == null
                  ? "—"
                  : `${number(summary.mean_ttft_ms).toFixed(0)}ms`
              }
              hint="First generated text, reasoning or tool output"
            />
            <Metric
              title="Aggregate output throughput"
              value={
                summary.output_tokens_per_second == null
                  ? "—"
                  : `${number(summary.output_tokens_per_second).toFixed(1)} tok/s`
              }
              hint="Reported output tokens ÷ request durations"
            />
          </div>
          <Tabs defaultValue="breakdown">
            <TabsList>
              <TabsTrigger value="breakdown">Breakdown</TabsTrigger>
              <TabsTrigger value="requests">Requests</TabsTrigger>
              <TabsTrigger value="sessions">Sessions</TabsTrigger>
            </TabsList>
            <TabsContent value="breakdown" className="space-y-4">
              <Select value={dimension} onValueChange={setDimension}>
                <SelectTrigger aria-label="Group usage by">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {[
                    "model",
                    "instance",
                    "token_id",
                    "integration",
                    "session_id",
                    "protocol",
                  ].map((value) => (
                    <SelectItem key={value} value={value}>
                      {value.replaceAll("_", " ")}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <DataTable
                rows={breakdown}
                columns={[
                  { key: dimension, label: "Group" },
                  {
                    key: "requests",
                    label: "Requests",
                    render: (row) => count(row.requests),
                  },
                  {
                    key: "input_tokens",
                    label: "Input tokens",
                    render: (row) => count(row.input_tokens),
                  },
                  {
                    key: "output_tokens",
                    label: "Output tokens",
                    render: (row) => count(row.output_tokens),
                  },
                  { key: "unknown_usage_requests", label: "Unknown usage" },
                ]}
              />
            </TabsContent>
            <TabsContent value="requests">
              <Paginated
                key={queryText}
                path={`analytics/requests?${queryText}`}
                columns={[
                  { key: "started_at", label: "Started" },
                  { key: "model", label: "Model" },
                  { key: "protocol", label: "Protocol" },
                  { key: "input_tokens", label: "Input" },
                  { key: "output_tokens", label: "Output" },
                  { key: "outcome", label: "Outcome" },
                ]}
              />
            </TabsContent>
            <TabsContent value="sessions">
              <Paginated
                key={queryText}
                path={`analytics/sessions?${queryText}`}
                columns={[
                  { key: "started_at", label: "Started" },
                  { key: "model", label: "Model" },
                  { key: "integration", label: "Integration" },
                  { key: "eco_mode", label: "Eco mode" },
                  { key: "finished_at", label: "Finished" },
                ]}
              />
            </TabsContent>
          </Tabs>
          <p className="text-xs leading-5 text-muted-foreground">
            Details are retained for 90 days; daily rollups for one year. Before
            the panel upgrade, retained history covers launched sessions only.
            No prompts, outputs or images are stored in analytics.
          </p>
        </>
      ) : (
        <Loading error={resource.error} />
      )}
    </div>
  );
}

function Panel({ session }: { session: Session }) {
  return (
    <SidebarProvider>
      <PanelLayout session={session} />
    </SidebarProvider>
  );
}
function PanelLayout({ session }: { session: Session }) {
  const { setOpenMobile } = useSidebar();
  const getPage = () => window.location.pathname.split("/")[2] || "overview";
  const [page, setPage] = useState(getPage);
  const status = useResource(
    permits(session, "instances:read") ? "status" : null,
    10000,
  );
  useEffect(() => {
    const pop = () => setPage(getPage());
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  const current = navigation.find((item) => item.id === page);
  const allowed =
    current && (!current.scope || permits(session, current.scope));
  const navigate = (value: string) => {
    window.history.pushState(null, "", `/panel/${value}`);
    setPage(value);
    setOpenMobile(false);
    window.scrollTo(0, 0);
  };
  return (
    <>
      <Sidebar collapsible="icon">
        <SidebarHeader className="px-4 py-6">
          <Logo />
        </SidebarHeader>
        <SidebarContent>
          <SidebarGroup>
            <SidebarGroupContent>
              <SidebarMenu>
                {navigation
                  .filter((item) => !item.scope || permits(session, item.scope))
                  .map((item) => (
                    <SidebarMenuItem key={item.id}>
                      <SidebarMenuButton
                        asChild
                        isActive={page === item.id}
                        tooltip={item.title}
                      >
                        <a
                          href={`/panel/${item.id}`}
                          onClick={(event) => {
                            if (
                              event.button === 0 &&
                              !event.metaKey &&
                              !event.ctrlKey
                            ) {
                              event.preventDefault();
                              navigate(item.id);
                            }
                          }}
                        >
                          <item.icon />
                          <span>{item.title}</span>
                        </a>
                      </SidebarMenuButton>
                    </SidebarMenuItem>
                  ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        </SidebarContent>
        <SidebarFooter className="p-4">
          <div className="flex items-center gap-2 rounded-lg border border-primary/10 bg-primary/5 p-3 text-xs">
            <span
              className={`status-dot ${status.error || status.data?.read_only !== false ? "!bg-amber-400" : ""}`}
            />
            <span className="truncate group-data-[collapsible=icon]:hidden">
              {!permits(session, "instances:read")
                ? "DGX Spark · scoped view"
                : status.error
                  ? "Connection interrupted"
                  : "DGX Spark · managed"}
            </span>
          </div>
          <span className="px-1 text-[10px] tracking-wider text-muted-foreground group-data-[collapsible=icon]:hidden">
            LOCAL INTELLIGENCE. VISIBLE VALUE.
          </span>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-w-0">
        <header className="flex h-16 items-center gap-3 border-b px-4 md:px-8">
          <SidebarTrigger />
          <span className="hidden min-w-0 truncate text-sm text-muted-foreground sm:block">
            Spark<span className="mx-3 text-border">/</span>
            <span className="text-foreground">{current?.title ?? "Panel"}</span>
          </span>
          <div className="ml-auto flex items-center gap-3">
            <div className="hidden sm:block">
              <StateBadge
                value={
                  !permits(session, "instances:read")
                    ? "scoped view"
                    : status.error || !status.data
                      ? "unavailable"
                      : status.data.read_only
                        ? "degraded"
                        : "healthy"
                }
              />
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm">
                  <ShieldCheck />
                  {session.scopes.includes("admin")
                    ? "Administrator"
                    : "Scoped access"}
                  <ChevronDown />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuLabel className="max-w-64 truncate">
                  {session.token_id}
                </DropdownMenuLabel>
                <DropdownMenuItem
                  onClick={() => {
                    void api("web-session", { method: "DELETE" })
                      .then(() => {
                        setCsrf(null);
                        window.location.assign("/panel/");
                      })
                      .catch((error) => toast.error(String(error)));
                  }}
                >
                  <LogOut />
                  Log out
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </header>
        <main className="mx-auto w-full max-w-[1600px] space-y-6 p-4 md:p-8">
          {!allowed ? (
            <PageTitle
              title="Access unavailable"
              description="This credential does not permit this page."
            />
          ) : page === "overview" ? (
            <Overview session={session} />
          ) : page === "analytics" ? (
            <Analytics />
          ) : page === "models" ? (
            <ModelsPage session={session} />
          ) : page === "instances" ? (
            <InstancesPage session={session} />
          ) : page === "operations" ? (
            <OperationsPage session={session} />
          ) : page === "health" ? (
            <HealthPage session={session} />
          ) : (
            <AdminPage session={session} />
          )}
        </main>
      </SidebarInset>
    </>
  );
}
export default function App() {
  const resource = useResource<Session>("web-session", 60000);
  const refreshSession = resource.refresh;
  useEffect(() => {
    setCsrf(resource.data?.csrf ?? null);
  }, [resource.data]);
  useEffect(() => {
    window.addEventListener("sparkplane:unauthenticated", refreshSession);
    return () =>
      window.removeEventListener("sparkplane:unauthenticated", refreshSession);
  }, [refreshSession]);
  const unauthenticated =
    resource.error instanceof ApiError && resource.error.status === 401;
  return (
    <TooltipProvider>
      {resource.data && !unauthenticated ? (
        <Panel session={resource.data} />
      ) : unauthenticated ? (
        <Login />
      ) : (
        <main className="grid-panel flex min-h-svh flex-col gap-10 p-8">
          <Logo />
          <div className="mx-auto w-full max-w-lg">
            <Loading error={resource.error} />
            {resource.error && (
              <Button
                variant="outline"
                className="mt-4"
                onClick={resource.refresh}
              >
                Reconnect
              </Button>
            )}
          </div>
        </main>
      )}
      <Toaster theme="dark" position="bottom-right" richColors />
    </TooltipProvider>
  );
}

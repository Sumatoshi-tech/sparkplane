import { useRef, useState, type ReactNode } from "react";
import { Copy, LoaderCircle, RefreshCw } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { api, records, text, useResource, type Doc } from "@/lib/api";

export function StateBadge({ value }: { value: unknown }) {
  const state = text(value);
  const good = [
    "healthy",
    "succeeded",
    "active",
    "ready",
    "running",
    "ok",
  ].includes(state);
  const bad = [
    "failed",
    "degraded",
    "revoked",
    "error",
    "unavailable",
  ].includes(state);
  return (
    <Badge
      variant="outline"
      className={
        good
          ? "border-primary/30 bg-primary/10 text-chart-2"
          : bad
            ? "border-destructive/30 bg-destructive/10 text-destructive"
            : "text-muted-foreground"
      }
    >
      {state.replaceAll("_", " ")}
    </Badge>
  );
}
export function Metric({
  title,
  value,
  hint,
}: {
  title: string;
  value: ReactNode;
  hint: string;
}) {
  return (
    <Card className="gap-3 py-5">
      <CardHeader className="pb-0">
        <CardDescription>{title}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="text-3xl font-semibold tracking-tight tabular-nums">
          {value}
        </div>
        <p className="mt-2 text-xs text-muted-foreground">{hint}</p>
      </CardContent>
    </Card>
  );
}
export function ErrorNotice({
  error,
  refresh,
}: {
  error?: Error;
  refresh?: () => void;
}) {
  if (!error) return null;
  return (
    <div
      role="alert"
      className="flex items-center justify-between gap-4 rounded-lg border border-amber-500/25 bg-amber-500/5 p-4 text-sm text-amber-200"
    >
      <span>{error.message}. Previously loaded data may be stale.</span>
      {refresh && (
        <Button size="sm" variant="outline" onClick={refresh}>
          <RefreshCw />
          Retry
        </Button>
      )}
    </div>
  );
}
export function Loading({ error }: { error?: Error }) {
  return error ? (
    <ErrorNotice error={error} />
  ) : (
    <div aria-label="Loading" className="grid gap-4 sm:grid-cols-3">
      {[1, 2, 3].map((key) => (
        <Skeleton key={key} className="h-32 rounded-lg" />
      ))}
    </div>
  );
}
export function JsonView({ value }: { value: unknown }) {
  return (
    <pre className="max-h-[55vh] overflow-auto rounded-lg bg-background p-4 text-xs leading-6 whitespace-pre-wrap break-all">
      {JSON.stringify(value, null, 2)}
    </pre>
  );
}
export function Details({
  title,
  value,
  path,
}: {
  title: string;
  value?: unknown;
  path?: string;
}) {
  const [open, setOpen] = useState(false);
  const resource = useResource(path && open ? path : null, 3000);
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button size="sm" variant="ghost">
          {title}
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>
            Current managed appliance information.
          </DialogDescription>
        </DialogHeader>
        {path ? (
          <>
            <ErrorNotice error={resource.error} refresh={resource.refresh} />
            <JsonView value={resource.data ?? "Loading…"} />
          </>
        ) : (
          <JsonView value={value} />
        )}
      </DialogContent>
    </Dialog>
  );
}
export function Code({ value }: { value: string }) {
  return (
    <div className="flex items-center gap-3 rounded-md border bg-background p-3">
      <code className="min-w-0 flex-1 overflow-x-auto whitespace-pre text-xs">
        {value}
      </code>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="Copy command"
        onClick={() => {
          void navigator.clipboard.writeText(value).then(
            () => toast.success("Copied"),
            () => toast.error("Clipboard unavailable"),
          );
        }}
      >
        <Copy />
      </Button>
    </div>
  );
}
export function DataTable({
  rows,
  columns,
  actions,
  empty = "No records in this period.",
}: {
  rows: Doc[];
  columns: { key: string; label: string; render?: (row: Doc) => ReactNode }[];
  actions?: (row: Doc) => ReactNode;
  empty?: string;
}) {
  return (
    <div className="overflow-hidden rounded-lg border">
      <Table>
        <TableHeader>
          <TableRow>
            {columns.map((column) => (
              <TableHead key={column.key}>{column.label}</TableHead>
            ))}
            {actions && <TableHead className="text-right">Actions</TableHead>}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.length === 0 ? (
            <TableRow>
              <TableCell
                colSpan={columns.length + (actions ? 1 : 0)}
                className="h-28 text-center text-muted-foreground"
              >
                {empty}
              </TableCell>
            </TableRow>
          ) : (
            rows.map((row, index) => (
              <TableRow key={text(row.id ?? row.sequence ?? index)}>
                {columns.map((column) => (
                  <TableCell key={column.key} className="max-w-72 truncate">
                    {column.render ? column.render(row) : text(row[column.key])}
                  </TableCell>
                ))}
                {actions && (
                  <TableCell>
                    <div className="flex justify-end gap-1">{actions(row)}</div>
                  </TableCell>
                )}
              </TableRow>
            ))
          )}
        </TableBody>
      </Table>
    </div>
  );
}
export function Action({
  title,
  description,
  children,
  prepare,
  execute,
  onDone,
  danger = false,
}: {
  title: string;
  description: string;
  children?: ReactNode;
  prepare?: () => Promise<unknown>;
  execute: (key: string) => Promise<Doc>;
  onDone?: () => void;
  danger?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<unknown>();
  const [error, setError] = useState<Error>();
  const key = useRef("");
  const act = async () => {
    setBusy(true);
    setError(undefined);
    try {
      if (prepare && preview === undefined) {
        setPreview(await prepare());
        return;
      }
      const result = await execute(key.current);
      const operation = result.id ?? result.operation_id;
      toast.success(
        operation
          ? `Operation accepted: ${text(operation)}`
          : "Action completed",
      );
      onDone?.();
      setOpen(false);
    } catch (error) {
      setError(error instanceof Error ? error : new Error(String(error)));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      open={open}
      onOpenChange={(value) => {
        setOpen(value);
        if (value) {
          key.current = crypto.randomUUID();
          setPreview(undefined);
          setError(undefined);
        }
      }}
    >
      <DialogTrigger asChild>
        <Button size="sm" variant={danger ? "outline" : "secondary"}>
          {title}
        </Button>
      </DialogTrigger>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
          <DialogDescription>{description}</DialogDescription>
        </DialogHeader>
        {preview === undefined ? (
          <fieldset disabled={busy}>{children}</fieldset>
        ) : (
          <JsonView value={preview} />
        )}
        <ErrorNotice error={error} />
        <DialogFooter>
          <Button
            variant="ghost"
            onClick={() => setOpen(false)}
            disabled={busy}
          >
            Cancel
          </Button>
          <Button
            variant={
              danger && (!prepare || preview !== undefined)
                ? "destructive"
                : "default"
            }
            disabled={busy}
            onClick={() => {
              void act();
            }}
          >
            {busy && <LoaderCircle className="animate-spin" />}
            {prepare && preview === undefined ? "Preview" : "Confirm"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
export function PageTitle({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-4">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      {action}
    </div>
  );
}
export function Paginated({
  path,
  columns,
}: {
  path: string;
  columns: { key: string; label: string }[];
}) {
  const [offset, setOffset] = useState(0);
  const resource = useResource(
    `${path}${path.includes("?") ? "&" : "?"}offset=${offset}&limit=50`,
  );
  return (
    <div className="space-y-4">
      <ErrorNotice error={resource.error} refresh={resource.refresh} />
      {resource.data ? (
        <DataTable
          rows={records(resource.data.items)}
          columns={columns}
          actions={(row) => <Details title="Details" value={row} />}
        />
      ) : (
        <Loading error={resource.error} />
      )}
      <div className="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          disabled={offset === 0}
          onClick={() => setOffset(Math.max(0, offset - 50))}
        >
          Previous
        </Button>
        <Button
          variant="outline"
          size="sm"
          disabled={resource.data?.next_offset == null}
          onClick={() => setOffset(offset + 50)}
        >
          Next
        </Button>
      </div>
    </div>
  );
}
export { api, Card, CardContent, CardHeader, CardTitle, CardDescription };

import { useCallback, useEffect, useState } from "react";

export type Doc = Record<string, unknown>;
export type Session = {
  token_id: string;
  scopes: string[];
  csrf: string | null;
};
export const object = (value: unknown): Doc =>
  value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Doc)
    : {};
export const records = (value: unknown): Doc[] =>
  Array.isArray(value) ? value.map(object) : [];
export const text = (value: unknown): string =>
  value == null
    ? "Unavailable"
    : typeof value === "object"
      ? JSON.stringify(value)
      : String(value);
export const number = (value: unknown): number =>
  typeof value === "number" && Number.isFinite(value) ? value : 0;
export const permits = (session: Session, scope: string): boolean =>
  session.scopes.includes("admin") || session.scopes.includes(scope);
export const count = (value: unknown): string =>
  value == null
    ? "—"
    : new Intl.NumberFormat("en", {
        notation: "compact",
        maximumFractionDigits: 1,
      }).format(number(value));
export const money = (nanos: unknown): string =>
  nanos == null
    ? "Unknown"
    : new Intl.NumberFormat("en", {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 2,
      }).format(number(nanos) / 1e9);
export const bytes = (value: unknown): string =>
  value == null
    ? "Unavailable"
    : `${(number(value) / 1024 ** 3).toFixed(1)} GiB`;
let csrf: string | null = null;
export function setCsrf(value: string | null) {
  csrf = value;
}
export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
  ) {
    super(message);
  }
}
export async function api<T = Doc>(
  path: string,
  options: {
    method?: string;
    body?: unknown;
    signal?: AbortSignal;
    key?: string;
  } = {},
): Promise<T> {
  const method = options.method ?? "GET";
  const headers: Record<string, string> = { Accept: "application/json" };
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  if (method !== "GET") {
    if (csrf) headers["X-Sparkplane-Csrf"] = csrf;
    headers["Idempotency-Key"] = options.key ?? crypto.randomUUID();
  }
  const response = await fetch(`/api/sparkplane/v1/${path}`, {
    method,
    headers,
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
    signal: options.signal,
    credentials: "same-origin",
    cache: "no-store",
  });
  if (!response.ok) {
    const problem = object(await response.json().catch(() => ({})));
    if (response.status === 401 && path !== "web-session")
      window.dispatchEvent(new Event("sparkplane:unauthenticated"));
    throw new ApiError(
      response.status,
      text(
        problem.detail ??
          problem.message ??
          `${response.status} ${response.statusText}`,
      ),
    );
  }
  return response.status === 204
    ? (undefined as T)
    : ((await response.json()) as T);
}
export function useResource<T = Doc>(path: string | null, interval = 10000) {
  const [result, setResult] = useState<{
    path: string;
    data?: T;
    error?: Error;
    updated: number;
  }>();
  const [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((value) => value + 1), []);
  useEffect(() => {
    if (!path) return;
    const controller = new AbortController();
    let active = true;
    let busy = false;
    const load = async () => {
      if (busy) return;
      busy = true;
      try {
        const value = await api<T>(path, { signal: controller.signal });
        if (active) {
          setResult({ path, data: value, updated: Date.now() });
        }
      } catch (error) {
        if (active && !controller.signal.aborted)
          setResult((previous) => ({
            path,
            data: previous?.path === path ? previous.data : undefined,
            updated: previous?.path === path ? previous.updated : 0,
            error: error instanceof Error ? error : new Error(String(error)),
          }));
      } finally {
        busy = false;
      }
    };
    void load();
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, interval);
    return () => {
      active = false;
      controller.abort();
      clearInterval(timer);
    };
  }, [path, interval, revision]);
  return {
    data: path && result?.path === path ? result.data : undefined,
    error: path && result?.path === path ? result.error : undefined,
    updated: path && result?.path === path ? result.updated : 0,
    refresh,
  };
}
export function csv(rows: Doc[], columns: string[]): string {
  const cell = (value: unknown) => {
    let valueText = value == null ? "" : text(value);
    // Exported data must not become a spreadsheet formula.
    if (/^[=+@\-\t\r]/.test(valueText)) valueText = `'${valueText}`;
    return `"${valueText.replaceAll('"', '""')}"`;
  };
  return [
    columns.map(cell).join(","),
    ...rows.map((row) => columns.map((key) => cell(row[key])).join(",")),
  ].join("\r\n");
}
export function exportCsv(rows: Doc[], columns: string[], filename: string) {
  const url = URL.createObjectURL(
    new Blob([csv(rows, columns)], { type: "text/csv;charset=utf-8" }),
  );
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

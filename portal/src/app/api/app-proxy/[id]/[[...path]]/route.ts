import { NextRequest, NextResponse } from "next/server";

const BACKEND_BASE =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "") ?? "trythissoftware.com"}`;

const READY_LOG_PATTERN = /\b(ready|listening|compiled|started server|vite v[\d.]+)\b/i;
const MIN_POLL_INTERVAL_MS = 100;
const MAX_PROBE_TIMEOUT_MS = 500;

type WorkspaceState = "Running" | "Failed" | string;

type WorkspaceInfo = {
  state?: WorkspaceState;
  framework?: string;
  ports?: Array<{ port?: number }>;
};

const FRAMEWORK_STARTUP_TIMEOUTS_MS: Record<string, number> = {
  vite: 30_000,
  react: 30_000,
  svelte: 30_000,
  node: 30_000,
  nextjs: 45_000,
  "next.js": 45_000,
  nuxt: 45_000,
  python: 45_000,
  angular: 60_000,
  django: 60_000,
  fastapi: 30_000,
};

function startupTimeoutMs(framework?: string): number {
  const normalized = framework?.toLowerCase().replace(/[_\s]+/g, "");
  if (!normalized) return 45_000;
  return FRAMEWORK_STARTUP_TIMEOUTS_MS[normalized] ?? 45_000;
}

function retryDelayMs(attempt: number): number {
  if (attempt < 2) return 500;
  if (attempt < 4) return 1_000;
  return 2_000;
}

function progressPercent(elapsedMs: number, maxWaitMs: number): number {
  if (maxWaitMs <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((elapsedMs / maxWaitMs) * 100)));
}

function shouldFetchLogs(attempt: number, hasLogs: boolean): boolean {
  // Fetch immediately, then every other probe to balance startup visibility and API load.
  return !hasLogs || attempt % 2 === 0;
}

function probeError(error: unknown): string {
  if (error instanceof Error && error.name === "TimeoutError") {
    return "timeout";
  }
  return error instanceof Error ? error.message : "connection failed";
}

function extractProcessInfo(logs: string[]): string {
  const pidLine = logs.find((line) => /spawned pid:/i.test(line));
  if (!pidLine) return "unknown";
  const match = pidLine.match(/spawned pid:\s*(\d+)/i);
  return match ? `running (pid ${match[1]})` : "running";
}

function validPort(port: number | null): port is number {
  return typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65_535;
}

async function getWorkspace(id: string): Promise<WorkspaceInfo | null> {
  try {
    const res = await fetch(`${BACKEND_BASE}/workspaces/${id}`, {
      cache: "no-store",
    });
    if (!res.ok) return null;
    return (await res.json()) as WorkspaceInfo;
  } catch {
    return null;
  }
}

async function getWorkspaceLogs(id: string): Promise<string[]> {
  try {
    const res = await fetch(`${BACKEND_BASE}/workspaces/${id}/logs`, {
      cache: "no-store",
    });
    if (!res.ok) return [];
    const data = (await res.json()) as { logs?: string[] };
    return Array.isArray(data.logs) ? data.logs : [];
  } catch {
    return [];
  }
}

async function handle(
  request: NextRequest,
  params: Promise<{ id: string; path?: string[] }>,
): Promise<NextResponse> {
  const { id, path } = await params;
  const startedAt = Date.now();
  let attempt = 0;
  let workspace = await getWorkspace(id);
  let lastLogs: string[] = [];
  let lastProbe = "not started";

  if (!workspace) {
    return NextResponse.json(
      { error: "Workspace not found" },
      { status: 404 },
    );
  }
  const maxWaitMs = startupTimeoutMs(workspace.framework);

  const subPath = path ? path.join("/") : "";
  const makeUpstreamUrl = (port: number): string =>
    `http://127.0.0.1:${port}/${subPath}${request.nextUrl.search}`;

  const forwardHeaders = new Headers();
  request.headers.forEach((value, key) => {
    if (!["host", "connection", "transfer-encoding"].includes(key.toLowerCase())) {
      forwardHeaders.set(key, value);
    }
  });

  const bodyBytes =
    request.method !== "GET" && request.method !== "HEAD"
      ? new Uint8Array(await request.arrayBuffer())
      : undefined;

  const deadline = startedAt + maxWaitMs;
  while (Date.now() < deadline) {
    const now = Date.now();
    const remaining = deadline - now;
    if (remaining < MIN_POLL_INTERVAL_MS) {
      break;
    }
    const port = workspace.ports?.[0]?.port ?? null;
    if (!validPort(port)) {
      lastProbe = "port unavailable";
    } else {
      forwardHeaders.set("host", `127.0.0.1:${port}`);
      try {
        const upstreamRes = await fetch(makeUpstreamUrl(port), {
          method: request.method,
          headers: forwardHeaders,
          body: bodyBytes,
          signal: AbortSignal.timeout(Math.min(MAX_PROBE_TIMEOUT_MS, remaining)),
        });

        const contentType = upstreamRes.headers.get("content-type") ?? "";
        const proxyBase = `/api/app-proxy/${id}/`;

        if (contentType.includes("text/html")) {
          let html = await upstreamRes.text();
          const rewriteAbsoluteToProxy = (value: string): string =>
            value
              .replace(
                /=\s*(["'])\/(?!\/)([^"']*)\1/g,
                (_match, quote: string, path: string) =>
                  `=${quote}${proxyBase}${path}${quote}`,
              )
              .replace(
                /=\s*\/(?!\/)([^\s"'=<>`]+)/g,
                (_match, path: string) => `=${proxyBase}${path}`,
              );

          html = rewriteAbsoluteToProxy(html);
          // Make relative URLs resolve through our proxy
          const baseTag = `<base href="${proxyBase}">`;
          if (html.includes("<head>")) {
            html = html.replace("<head>", `<head>${baseTag}`);
          } else {
            html = baseTag + html;
          }
          return new NextResponse(html, {
            status: upstreamRes.status,
            headers: { "content-type": "text/html; charset=utf-8" },
          });
        }

        const responseHeaders = new Headers();
        responseHeaders.set("content-type", contentType || "application/octet-stream");
        const cacheControl = upstreamRes.headers.get("cache-control");
        if (cacheControl) responseHeaders.set("cache-control", cacheControl);

        return new NextResponse(upstreamRes.body, {
          status: upstreamRes.status,
          headers: responseHeaders,
        });
      } catch (error) {
        lastProbe = probeError(error);
      }
    }

    if (workspace.state === "Failed") {
      const elapsed = Date.now() - startedAt;
      const logs = await getWorkspaceLogs(id);
      return NextResponse.json(
        {
          error: "Workspace failed to become ready.",
          status: "failed",
          framework: workspace.framework ?? "unknown",
          process: extractProcessInfo(logs),
          workspaceState: workspace.state,
          port: workspace.ports?.[0]?.port ?? null,
          lastProbe,
          startupElapsedSeconds: Math.floor(elapsed / 1000),
          startupMaxSeconds: Math.floor(maxWaitMs / 1000),
          logs: logs.slice(-5),
        },
        { status: 502 },
      );
    }

    if (shouldFetchLogs(attempt, lastLogs.length > 0)) {
      const logs = await getWorkspaceLogs(id);
      if (logs.length > 0) {
        lastLogs = logs.slice(-5);
        if (lastLogs.some((line) => READY_LOG_PATTERN.test(line))) {
          lastProbe = "startup logs indicate server may be ready";
        }
      }
    }

    const delay = retryDelayMs(attempt);
    const elapsed = Date.now() - startedAt;
    const remainingAfterWork = maxWaitMs - elapsed;
    if (remainingAfterWork < MIN_POLL_INTERVAL_MS) {
      break;
    }
    const waitMs = Math.max(
      MIN_POLL_INTERVAL_MS,
      Math.min(delay, remainingAfterWork),
    );
    await new Promise((r) => setTimeout(r, waitMs));
    attempt += 1;
    const refreshed = await getWorkspace(id);
    if (!refreshed) {
      break;
    }
    workspace = refreshed;
  }

  const elapsed = Date.now() - startedAt;
  return NextResponse.json(
    {
      status: "starting",
      framework: workspace.framework ?? "unknown",
      process: extractProcessInfo(lastLogs),
      workspaceState: workspace.state ?? "unknown",
      port: workspace.ports?.[0]?.port ?? null,
      progress: progressPercent(elapsed, maxWaitMs),
      lastProbe,
      startupElapsedSeconds: Math.floor(elapsed / 1000),
      startupMaxSeconds: Math.floor(maxWaitMs / 1000),
      logs: lastLogs,
    },
    { status: 202 },
  );
}

export async function GET(
  req: NextRequest,
  ctx: { params: Promise<{ id: string; path?: string[] }> },
) {
  return handle(req, ctx.params);
}

export async function POST(
  req: NextRequest,
  ctx: { params: Promise<{ id: string; path?: string[] }> },
) {
  return handle(req, ctx.params);
}

export async function PUT(
  req: NextRequest,
  ctx: { params: Promise<{ id: string; path?: string[] }> },
) {
  return handle(req, ctx.params);
}

export async function DELETE(
  req: NextRequest,
  ctx: { params: Promise<{ id: string; path?: string[] }> },
) {
  return handle(req, ctx.params);
}

import { NextRequest, NextResponse } from "next/server";

const BACKEND_BASE =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "") ?? "trythissoftware.com"}`;

const MAX_PROBE_TIMEOUT_MS = 500;

type WorkspaceRuntime = {
  framework?: string;
  workspace_id?: string;
  provider_selected?: string;
  pid?: number;
  alive?: boolean;
  process_state?: string;
  exit_code?: number | null;
  requested_port?: number;
  actual_port?: number | null;
  listening?: boolean;
  detected_start_signal?: string | null;
  http_ready?: boolean;
  readiness_state?: string;
  lifecycle_state?: string;
  last_http_probe?: string;
  last_probe?: string;
  stdout?: string[];
  stderr?: string[];
};

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;" }[c] ?? c));
}

async function getRuntime(id: string): Promise<WorkspaceRuntime | null> {
  try {
    const res = await fetch(`${BACKEND_BASE}/workspaces/${id}/runtime`, { cache: "no-store" });
    if (!res.ok) return null;
    return (await res.json()) as WorkspaceRuntime;
  } catch {
    return null;
  }
}

function startupHtml(id: string, runtime: WorkspaceRuntime): string {
  const logs = [...(runtime.stdout ?? []), ...(runtime.stderr ?? [])].slice(-20);
  const safeLogs = logs
    .map((line) => escapeHtml(line))
    .join("\n");
  const details = [
    `Workspace: ${escapeHtml(id)}`,
    `Framework: ${escapeHtml(runtime.framework ?? "unknown")}`,
    `Status: ${escapeHtml(runtime.lifecycle_state ?? "Initializing")}`,
    `PID: ${escapeHtml(String(runtime.pid ?? "unknown"))}`,
    `Probe: ${escapeHtml(runtime.last_http_probe ?? runtime.last_probe ?? "connection refused")}`,
  ]
    .map((line) => `<div>${line}</div>`)
    .join("");
  return `<!doctype html><html><head><meta charset="utf-8"/><meta http-equiv="refresh" content="1"><title>Starting Workspace</title></head><body style="font-family: ui-monospace, SFMono-Regular, Menlo, monospace; padding: 24px;"><h2>🚀 Starting Workspace</h2>${details}<hr/><pre style="white-space: pre-wrap;">${safeLogs}</pre></body></html>`;
}

async function handle(
  request: NextRequest,
  params: Promise<{ id: string; path?: string[] }>,
): Promise<NextResponse> {
  const { id, path } = await params;
  const runtime = await getRuntime(id);
  const truth = runtime ?? {};
  const port = truth.actual_port ?? null;
  const isReady = Boolean(truth.http_ready && port);

  if (!isReady || !port) {
    const payload = {
      workspaceId: id,
      framework: truth.framework ?? "unknown",
      state: truth.lifecycle_state ?? "Initializing",
      provider: truth.provider_selected ?? null,
      processState: truth.process_state ?? null,
      readinessState: truth.readiness_state ?? null,
      pid: truth.pid ?? null,
      requestedPort: truth.requested_port ?? null,
      actualPort: truth.actual_port ?? null,
      processAlive: truth.alive ?? false,
      httpReady: truth.http_ready ?? false,
      lastProbe: truth.last_http_probe ?? truth.last_probe ?? "connection refused",
      detectedStartSignal: truth.detected_start_signal ?? null,
      logs: [...(truth.stdout ?? []), ...(truth.stderr ?? [])].slice(-20),
      truth,
    };

    if (request.method === "GET") {
      return new NextResponse(startupHtml(id, truth), {
        status: 202,
        headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
      });
    }
    return NextResponse.json(payload, { status: 202 });
  }

  const subPath = path ? path.join("/") : "";
  const upstreamUrl = `http://127.0.0.1:${port}/${subPath}${request.nextUrl.search}`;
  const forwardHeaders = new Headers();
  request.headers.forEach((value, key) => {
    if (!["host", "connection", "transfer-encoding"].includes(key.toLowerCase())) {
      forwardHeaders.set(key, value);
    }
  });
  forwardHeaders.set("host", `127.0.0.1:${port}`);
  const body =
    request.method !== "GET" && request.method !== "HEAD"
      ? new Uint8Array(await request.arrayBuffer())
      : undefined;
  const upstreamRes = await fetch(upstreamUrl, {
    method: request.method,
    headers: forwardHeaders,
    body,
    signal: AbortSignal.timeout(MAX_PROBE_TIMEOUT_MS),
  });
  return new NextResponse(upstreamRes.body, {
    status: upstreamRes.status,
    headers: upstreamRes.headers,
  });
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

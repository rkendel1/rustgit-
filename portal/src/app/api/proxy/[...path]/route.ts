import { NextRequest, NextResponse } from "next/server";

const PRODUCTION_BASE_DOMAIN =
  process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "").replace(/\/.*$/, "") ??
  "trythissoftware.com";

const DEFAULT_API_BASE_URL =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${PRODUCTION_BASE_DOMAIN}`;
const UPSTREAM_PROXY_PREFIX_SEGMENTS = ["api", "proxy"] as const;
const CORS_ALLOW_METHODS = "GET, POST, DELETE, OPTIONS";
const CORS_ALLOW_HEADERS = "Content-Type, Authorization";

// Default timeout for most proxy calls.
const DEFAULT_TIMEOUT_MS = 30_000;
// Analyze clones the repo + detects frameworks — allow up to 3 minutes on slow hosts.
const ANALYZE_TIMEOUT_MS = 180_000;

function timeoutForPath(path: string): number {
  const normalized = path.replace(/^\/+/, "").toLowerCase();
  if (normalized === "api/analyze" || normalized.endsWith("/analyze")) {
    return ANALYZE_TIMEOUT_MS;
  }
  return DEFAULT_TIMEOUT_MS;
}

function resolveApiBaseUrl(request: NextRequest): string {
  const configuredApiUrl = process.env.NEXT_PUBLIC_API_URL;
  if (!configuredApiUrl) {
    return DEFAULT_API_BASE_URL;
  }

  try {
    const configured = new URL(configuredApiUrl);
    if (
      process.env.NODE_ENV !== "development" &&
      configured.hostname === request.nextUrl.hostname
    ) {
      return DEFAULT_API_BASE_URL;
    }
    return configured.toString().replace(/\/$/, "");
  } catch {
    return DEFAULT_API_BASE_URL;
  }
}

function appendSearchParams(source: URLSearchParams, target: URL): void {
  source.forEach((value, key) => {
    target.searchParams.append(key, value);
  });
}

function buildUpstreamUrl(apiBaseUrl: string, path: string): URL {
  const url = new URL(apiBaseUrl);
  const basePath = url.pathname.replace(/\/+$/, "");
  const normalizedPath = path.replace(/^\/+/, "");
  url.pathname = `${basePath}/${normalizedPath}`.replace(/\/{2,}/g, "/");
  return url;
}

function isTimeoutError(error: unknown): boolean {
  if (!error || typeof error !== "object") {
    return false;
  }
  if (error instanceof DOMException) {
    return error.name === "AbortError" || error.name === "TimeoutError";
  }
  return error instanceof Error && (error.name === "AbortError" || error.name === "TimeoutError");
}

function upstreamFailureResponse(error: unknown, path: string): NextResponse {
  const isTimeout = isTimeoutError(error);
  if (isTimeout) {
    const timeoutMs = timeoutForPath(path);
    console.warn(
      `Proxy upstream request timed out after ${timeoutMs}ms for path: ${path}`,
    );
  }
  return withCorsHeaders(
    NextResponse.json(
    {
      error: isTimeout ? "Upstream request timed out." : "Upstream request failed.",
      path,
    },
    { status: isTimeout ? 504 : 502 },
    ),
  );
}

function withCorsHeaders(response: NextResponse): NextResponse {
  response.headers.set("Access-Control-Allow-Origin", "*");
  response.headers.set("Access-Control-Allow-Methods", CORS_ALLOW_METHODS);
  response.headers.set("Access-Control-Allow-Headers", CORS_ALLOW_HEADERS);
  response.headers.set("Access-Control-Max-Age", "3600");
  return response;
}

async function proxyRequest(
  request: NextRequest,
  params: Promise<{ path: string[] }>,
): Promise<NextResponse> {
  const resolvedParams = await params;
  const joinedPath = resolvedParams.path
    .join("/")
    .replace(/\/{2,}/g, "/")
    .replace(/^\/+|\/+$/g, "");
  const apiBaseUrl = resolveApiBaseUrl(request);
  const upstreamUrl = buildUpstreamUrl(apiBaseUrl, joinedPath);

  appendSearchParams(request.nextUrl.searchParams, upstreamUrl);

  const requestHeaders = new Headers();
  const contentType = request.headers.get("content-type");
  if (contentType) {
    requestHeaders.set("content-type", contentType);
  }

  const authorization = request.headers.get("authorization");
  if (authorization) {
    requestHeaders.set("authorization", authorization);
  }

  const requestBodyBytes =
    request.method === "GET" || request.method === "HEAD"
      ? undefined
      : new Uint8Array(await request.arrayBuffer());

  const requestTimeoutMs = timeoutForPath(joinedPath);
  const sendUpstreamRequest = async (url: URL): Promise<Response> => {
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), requestTimeoutMs);
    try {
      return await fetch(url, {
        method: request.method,
        headers: requestHeaders,
        body: requestBodyBytes,
        cache: "no-store",
        signal: controller.signal,
      });
    } finally {
      clearTimeout(timeoutId);
    }
  };
  let upstreamResponse: Response;
  try {
    upstreamResponse = await sendUpstreamRequest(upstreamUrl);
  } catch (error) {
    return upstreamFailureResponse(error, joinedPath);
  }
  const joinedSegments = joinedPath.split("/");
  const hasProxyPrefix =
    joinedSegments.length >= 2 &&
    joinedSegments[0] === UPSTREAM_PROXY_PREFIX_SEGMENTS[0] &&
    joinedSegments[1] === UPSTREAM_PROXY_PREFIX_SEGMENTS[1];
  const canRetryWithProxyPrefix = !hasProxyPrefix;
  if (upstreamResponse.status === 404 && canRetryWithProxyPrefix) {
    const proxiedUpstreamUrl = buildUpstreamUrl(
      apiBaseUrl,
      `${UPSTREAM_PROXY_PREFIX_SEGMENTS.join("/")}/${joinedPath}`,
    );
    appendSearchParams(request.nextUrl.searchParams, proxiedUpstreamUrl);
    try {
      upstreamResponse = await sendUpstreamRequest(proxiedUpstreamUrl);
    } catch (error) {
      return upstreamFailureResponse(error, joinedPath);
    }
  }

  return withCorsHeaders(
    new NextResponse(upstreamResponse.body, {
      status: upstreamResponse.status,
      headers: {
        "content-type":
          upstreamResponse.headers.get("content-type") ?? "application/json",
      },
    }),
  );
}

export async function GET(
  request: NextRequest,
  context: { params: Promise<{ path: string[] }> },
) {
  return proxyRequest(request, context.params);
}

export async function POST(
  request: NextRequest,
  context: { params: Promise<{ path: string[] }> },
) {
  return proxyRequest(request, context.params);
}

export async function DELETE(
  request: NextRequest,
  context: { params: Promise<{ path: string[] }> },
) {
  return proxyRequest(request, context.params);
}

export async function OPTIONS() {
  return withCorsHeaders(new NextResponse(null, { status: 204 }));
}

import { NextRequest, NextResponse } from "next/server";
import { resolveLegacyFallbackPath } from "./legacy-path-fallback";

const PRODUCTION_BASE_DOMAIN =
  process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "").replace(/\/.*$/, "") ??
  "trythissoftware.com";

const DEFAULT_API_BASE_URL =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${PRODUCTION_BASE_DOMAIN}`;
const UPSTREAM_PROXY_PREFIX_SEGMENTS = ["api", "proxy"] as const;
const ALLOWED_WEB_ORIGINS = new Set([
  "https://trythissoftware.com",
  "https://www.trythissoftware.com",
  "https://rustgit.fly.dev",
]);
const CORS_ALLOW_METHODS = "GET, POST, DELETE, OPTIONS";
const CORS_ALLOW_HEADERS = "Content-Type, Authorization";

// Default timeout for most proxy calls.
const DEFAULT_TIMEOUT_MS = 30_000;
// Analyze clones the repo + detects frameworks — allow close to Fly's 5-minute request ceiling.
const ANALYZE_TIMEOUT_MS = 295_000;

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

function resolveAllowedOrigin(request: NextRequest): string | null {
  const origin = request.headers.get("origin");
  if (!origin) {
    return null;
  }

  if (origin === request.nextUrl.origin) {
    return origin;
  }

  if (process.env.NODE_ENV === "development") {
    try {
      const { hostname } = new URL(origin);
      if (hostname === "localhost" || hostname === "127.0.0.1") {
        return origin;
      }
    } catch { /* ignore invalid origin */ }
  }

  if (ALLOWED_WEB_ORIGINS.has(origin)) {
    return origin;
  }

  if (
    origin.startsWith("chrome-extension://") ||
    origin.startsWith("moz-extension://") ||
    origin.startsWith("safari-web-extension://")
  ) {
    return origin;
  }

  return null;
}

function upstreamFailureResponse(
  error: unknown,
  path: string,
  allowedOrigin: string | null,
): NextResponse {
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
    allowedOrigin,
  );
}

function withCorsHeaders(response: NextResponse, allowedOrigin: string | null): NextResponse {
  if (!allowedOrigin) {
    return response;
  }
  response.headers.set("Access-Control-Allow-Origin", allowedOrigin);
  response.headers.set("Access-Control-Allow-Methods", CORS_ALLOW_METHODS);
  response.headers.set("Access-Control-Allow-Headers", CORS_ALLOW_HEADERS);
  response.headers.set("Access-Control-Max-Age", "3600");
  response.headers.set("Vary", "Origin");
  return response;
}

function forbiddenOriginResponse(origin: string): NextResponse {
  return NextResponse.json(
    { error: "Origin is not allowed.", origin },
    { status: 403 },
  );
}

function resolveOriginDecision(request: NextRequest): {
  allowedOrigin: string | null;
  requestOrigin: string | null;
  isAllowed: boolean;
} {
  const requestOrigin = request.headers.get("origin");
  const allowedOrigin = resolveAllowedOrigin(request);
  const isAllowed = !requestOrigin || Boolean(allowedOrigin);
  return { allowedOrigin, requestOrigin, isAllowed };
}

async function proxyRequest(
  request: NextRequest,
  params: Promise<{ path: string[] }>,
): Promise<NextResponse> {
  const resolvedParams = await params;
  const originDecision = resolveOriginDecision(request);
  if (!originDecision.isAllowed && originDecision.requestOrigin) {
    return forbiddenOriginResponse(originDecision.requestOrigin);
  }

  const joinedPath = resolvedParams.path
    .join("/")
    .replace(/\/{2,}/g, "/")
    .replace(/^\/+|\/+$/g, "");
  const legacyFallbackPath = resolveLegacyFallbackPath(joinedPath);
  const apiBaseUrl = resolveApiBaseUrl(request);

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
  const requestPath = (path: string): Promise<Response> => {
    const upstreamUrl = buildUpstreamUrl(apiBaseUrl, path);
    appendSearchParams(request.nextUrl.searchParams, upstreamUrl);
    return sendUpstreamRequest(upstreamUrl);
  };
  let upstreamResponse: Response;
  try {
    upstreamResponse = await requestPath(joinedPath);
  } catch (error) {
    if (!legacyFallbackPath) {
      return upstreamFailureResponse(error, joinedPath, originDecision.allowedOrigin);
    }
    try {
      upstreamResponse = await requestPath(legacyFallbackPath);
    } catch (fallbackError) {
      console.warn("Proxy legacy fallback request failed after upstream error", {
        path: joinedPath,
        fallbackPath: legacyFallbackPath,
        error: fallbackError,
      });
      return upstreamFailureResponse(error, joinedPath, originDecision.allowedOrigin);
    }
  }
  if (upstreamResponse.status === 404 && legacyFallbackPath) {
    try {
      upstreamResponse = await requestPath(legacyFallbackPath);
    } catch (error) {
      console.warn("Proxy legacy fallback request failed after 404 response", {
        path: joinedPath,
        fallbackPath: legacyFallbackPath,
        error,
      });
      return upstreamFailureResponse(error, joinedPath, originDecision.allowedOrigin);
    }
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
      return upstreamFailureResponse(error, joinedPath, originDecision.allowedOrigin);
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
    originDecision.allowedOrigin,
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

export async function OPTIONS(request: NextRequest) {
  const originDecision = resolveOriginDecision(request);
  if (!originDecision.allowedOrigin) {
    if (!originDecision.requestOrigin) {
      return new NextResponse(null, { status: 204 });
    }
    return forbiddenOriginResponse(originDecision.requestOrigin);
  }
  return withCorsHeaders(new NextResponse(null, { status: 204 }), originDecision.allowedOrigin);
}

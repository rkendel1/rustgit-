import { NextRequest, NextResponse } from "next/server";

const PRODUCTION_BASE_DOMAIN =
  process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "").replace(/\/.*$/, "") ??
  "trythissoftware.com";

const DEFAULT_API_BASE_URL =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${PRODUCTION_BASE_DOMAIN}`;

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

async function proxyRequest(
  request: NextRequest,
  params: Promise<{ path: string[] }>,
): Promise<NextResponse> {
  const resolvedParams = await params;
  const joinedPath = resolvedParams.path.join("/").replace(/^\/+/, "");
  const apiBaseUrl = resolveApiBaseUrl(request);
  const upstreamUrl = new URL(`${apiBaseUrl}/${joinedPath}`);

  request.nextUrl.searchParams.forEach((value, key) => {
    upstreamUrl.searchParams.append(key, value);
  });

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

  const sendUpstreamRequest = (url: URL) =>
    fetch(url, {
      method: request.method,
      headers: requestHeaders,
      body: requestBodyBytes,
      cache: "no-store",
    });

  let upstreamResponse = await sendUpstreamRequest(upstreamUrl);
  const canRetryWithProxyPrefix = !joinedPath.startsWith("api/proxy/");
  if (upstreamResponse.status === 404 && canRetryWithProxyPrefix) {
    const proxiedUpstreamUrl = new URL(`${apiBaseUrl}/api/proxy/${joinedPath}`);
    request.nextUrl.searchParams.forEach((value, key) => {
      proxiedUpstreamUrl.searchParams.append(key, value);
    });
    upstreamResponse = await sendUpstreamRequest(proxiedUpstreamUrl);
  }

  return new NextResponse(upstreamResponse.body, {
    status: upstreamResponse.status,
    headers: {
      "content-type":
        upstreamResponse.headers.get("content-type") ?? "application/json",
    },
  });
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

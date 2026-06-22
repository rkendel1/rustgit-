import { chromium, type Browser, type Page } from "playwright";
import { NextResponse } from "next/server";

export const runtime = "nodejs";

const BACKEND_BASE =
  process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : `https://api.${process.env.NEXT_PUBLIC_BASE_DOMAIN?.replace(/^https?:\/\//, "") ?? "trythissoftware.com"}`;
let browserPromise: Promise<Browser> | null = null;

type WorkspaceInfo = {
  state?: string;
  ports?: Array<{ port?: number }>;
};

async function getWorkspace(id: string): Promise<WorkspaceInfo | null> {
  try {
    const response = await fetch(`${BACKEND_BASE}/workspaces/${id}`, {
      cache: "no-store",
    });
    if (!response.ok) {
      return null;
    }

    return (await response.json()) as WorkspaceInfo;
  } catch {
    return null;
  }
}

async function getBrowser(): Promise<Browser> {
  if (!browserPromise) {
    browserPromise = chromium.launch({ headless: true }).catch((error) => {
      browserPromise = null;
      throw error;
    });
  }
  return browserPromise;
}

export async function GET(
  request: Request,
  context: { params: Promise<{ id: string }> },
) {
  const { id } = await context.params;
  const workspace = await getWorkspace(id);
  const port = workspace?.ports?.[0]?.port;

  if (!workspace || workspace.state !== "Running" || !port) {
    return NextResponse.json(
      { error: "Workspace is not running yet." },
      { status: 409 },
    );
  }
  const proxyUrl = new URL(`/api/app-proxy/${id}/`, request.url).toString();
  const readiness = await fetch(proxyUrl, {
    cache: "no-store",
    signal: AbortSignal.timeout(2_000),
  });
  if (!readiness.ok) {
    let payload: unknown = null;
    try {
      payload = await readiness.json();
    } catch {
      payload = null;
    }
    return NextResponse.json(
      payload ?? { error: "Workspace app is not ready yet." },
      { status: readiness.status },
    );
  }

  let page: Page | null = null;
  try {
    const browser = await getBrowser();
    page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
    await page.goto(proxyUrl, {
      waitUntil: "domcontentloaded",
      timeout: 15_000,
    });
    await page.waitForLoadState("networkidle", { timeout: 15_000 });
    const image = await page.screenshot({ type: "png" });
    return new NextResponse(new Uint8Array(image), {
      status: 200,
      headers: {
        "content-type": "image/png",
        "cache-control": "no-store, max-age=0",
      },
    });
  } catch (error) {
    console.error("workspace preview screenshot failed", error);
    const message =
      error instanceof Error && error.name === "TimeoutError"
        ? "Workspace app did not finish loading within 15 seconds."
        // Keep this fallback to handle brief races between successful readiness checks
        // and browser navigation while the upstream process is still finalizing binds.
        : error instanceof Error && error.message.includes("ERR_CONNECTION_REFUSED")
          ? "Workspace app is still starting. Please retry in a few seconds."
        : "Failed to capture preview screenshot.";
    return NextResponse.json(
      { error: message },
      { status: 503 },
    );
  } finally {
    if (page) {
      await page.close();
    }
  }
}

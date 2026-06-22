"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import styles from "./page.module.css";

const API_BASE_URL =
  process.env.NEXT_PUBLIC_API_URL ??
  (process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : "https://api.trythissoftware.com");

const DEFAULT_ASK_QUESTION = "Summarize what this repository does and the best way to run it.";
const SCORE_DECIMAL_PLACES = 1;
const CONFIDENCE_DECIMAL_PLACES = 2;
const PORTAL_NAME = "RustGit Portal";
const NO_REPOSITORY_SELECTED = "No repository selected";
const DEFAULT_AVATAR_LETTER = "R";
const EMPTY_STATE_HEADING = "It's empty here";
const PORTAL_DEVICE_FINGERPRINT = "portal-home";
const ANALYZE_PATH = "/api/proxy/api/analyze";

type RepoContext = {
  owner: string;
  repo: string;
  repoUrl: string;
};

type AnalyzeResponse = {
  repo_url?: string;
  fingerprint_id?: string;
  frameworks?: string[];
  services?: string[];
};

type RunResponse = {
  execution_id?: string;
  workspace_id?: string;
  workspace_url?: string;
  status?: string;
};

type WorkspaceState =
  | "Created" | "Materializing" | "Analyzing" | "Planning" | "Pending"
  | "Provisioning" | "Starting" | "Running" | "Degraded" | "Restarting"
  | "Migrating" | "Paused" | "Failed" | "Stopping" | "Stopped" | "Destroyed";

const ACTIVE_WORKSPACE_STATES = new Set<WorkspaceState>([
  "Created", "Materializing", "Analyzing", "Planning",
  "Provisioning", "Starting", "Running", "Restarting",
]);

type Workspace = {
  id: string;
  repo_url: string;
  state: WorkspaceState;
  framework: string;
  ports: { port: number; protocol: string; route: string }[];
  resource_quotas: { max_memory_mb: number; max_cpu_millis: number };
};

type RepositoryIdentity = {
  health_score?: number;
  execution_score?: number;
  healing_score?: number;
};

type RepositoryIntelligenceResponse = {
  repository_id?: string;
  health_score?: number;
  execution_score?: number;
  healing_score?: number;
  runtime?: string;
  repository_identity?: RepositoryIdentity | null;
};

type RepositoryAskResponse = {
  answer?: string;
  confidence?: number;
  evidence?: string[];
};

function createAnonymousId(prefix: string): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return `${prefix}-${crypto.randomUUID()}`;
  }
  if (typeof crypto !== "undefined" && typeof crypto.getRandomValues === "function") {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
    return `${prefix}-${hex}`;
  }
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function parseRepositoryInput(input: string): RepoContext | null {
  const trimmed = input.trim();
  if (!trimmed) {
    return null;
  }

  if (trimmed.startsWith("http://") || trimmed.startsWith("https://")) {
    try {
      const url = new URL(trimmed);
      if (url.hostname !== "github.com" && url.hostname !== "www.github.com") {
        return null;
      }
      const segments = url.pathname
        .replace(/\.git$/, "")
        .split("/")
        .filter(Boolean);
      if (segments.length < 2) {
        return null;
      }
      const owner = segments[0];
      const repo = segments[1];
      const username = url.username;
      const password = url.password;
      const credentials =
        username || password
          ? `${username}${password ? `:${password}` : ""}@`
          : "";
      return {
        owner,
        repo,
        repoUrl: `https://${credentials}github.com/${owner}/${repo}.git`,
      };
    } catch {
      return null;
    }
  }

  const segments = trimmed
    .replace(/\.git$/, "")
    .split("/")
    .map((segment) => segment.trim())
    .filter(Boolean);
  if (segments.length !== 2) {
    return null;
  }

  const [owner, repo] = segments;
  return {
    owner,
    repo,
    repoUrl: `https://github.com/${owner}/${repo}.git`,
  };
}

async function readJsonResponse<T>(response: Response): Promise<T> {
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`Request failed (${response.status}): ${text || "no response body"}`);
  }
  try {
    return JSON.parse(text) as T;
  } catch (error) {
    const message = error instanceof Error ? error.message : "unknown parse error";
    throw new Error(`Invalid JSON response: ${message}`);
  }
}

export default function Home() {
  const [repository, setRepository] = useState("");
  const [branch, setBranch] = useState("main");
  const [analyzing, setAnalyzing] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [analyzeResult, setAnalyzeResult] = useState<AnalyzeResponse | null>(null);
  const [analyzedRepoUrl, setAnalyzedRepoUrl] = useState<string | null>(null);
  const [intelligence, setIntelligence] = useState<RepositoryIntelligenceResponse | null>(null);
  const [repoAnswer, setRepoAnswer] = useState<RepositoryAskResponse | null>(null);
  const [runResult, setRunResult] = useState<RunResponse | null>(null);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [workspaceLogs, setWorkspaceLogs] = useState<string[]>([]);
  const logBoxRef = useRef<HTMLDivElement>(null);
  const anonymousIdentity = useMemo(
    () => ({
      anonUserId: createAnonymousId("anon-portal"),
      anonSessionId: createAnonymousId("portal-session"),
    }),
    [],
  );

  const parsedRepo = useMemo(() => parseRepositoryInput(repository), [repository]);
  const canAnalyze = Boolean(parsedRepo) && !analyzing;
  const canRun =
    !running &&
    Boolean(parsedRepo) &&
    Boolean(analyzeResult) &&
    analyzedRepoUrl === parsedRepo?.repoUrl;

  function resetResults(nextRepositoryValue: string) {
    setRepository(nextRepositoryValue);
    setAnalyzeResult(null);
    setAnalyzedRepoUrl(null);
    setIntelligence(null);
    setRepoAnswer(null);
    setRunResult(null);
    setWorkspace(null);
    setWorkspaceLogs([]);
    setError(null);
  }

  const fetchWorkspaceData = useCallback(async (wsId: string) => {
    const [wsRes, logsRes] = await Promise.all([
      fetch(`/api/proxy/workspaces/${wsId}`, { cache: "no-store" }),
      fetch(`/api/proxy/workspaces/${wsId}/logs`, { cache: "no-store" }),
    ]);
    const ws: Workspace = await wsRes.json();
    const logsBody: { logs: string[] } = logsRes.ok ? await logsRes.json() : { logs: [] };
    setWorkspace(ws);
    setWorkspaceLogs(logsBody.logs ?? []);
    return ws;
  }, []);

  // Poll workspace when a run is in flight
  useEffect(() => {
    const wsId = runResult?.execution_id;
    if (!wsId) return;
    let cancelled = false;

    async function poll() {
      try {
        const ws = await fetchWorkspaceData(wsId!);
        if (cancelled) return;
        if (ACTIVE_WORKSPACE_STATES.has(ws.state)) {
          setTimeout(poll, 3000);
        }
      } catch {
        // silently retry
        if (!cancelled) setTimeout(poll, 3000);
      }
    }

    poll();
    return () => { cancelled = true; };
  }, [runResult?.execution_id, fetchWorkspaceData]);

  // Auto-scroll log box
  useEffect(() => {
    if (logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [workspaceLogs]);

  function scoreValue(
    identityScore: number | undefined,
    responseScore: number | undefined,
  ): number | null {
    if (typeof identityScore === "number") {
      return identityScore;
    }
    if (typeof responseScore === "number") {
      return responseScore;
    }
    return null;
  }

  function formatScore(score: number | null): string {
    return score === null ? "n/a" : score.toFixed(SCORE_DECIMAL_PLACES);
  }

  function formatConfidence(value: number | undefined): string {
    return typeof value === "number" ? value.toFixed(CONFIDENCE_DECIMAL_PLACES) : "n/a";
  }

  async function handleAnalyze() {
    if (!parsedRepo) {
      setError("Paste a GitHub repository URL or owner/repo.");
      return;
    }

    setError(null);
    setAnalyzing(true);
    setAnalyzeResult(null);
    setAnalyzedRepoUrl(null);
    setIntelligence(null);
    setRepoAnswer(null);
    setRunResult(null);

    try {
      const analyzeRequest = {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          org_id: null,
          user_id: null,
          anon_user_id: anonymousIdentity.anonUserId,
          anon_session_id: anonymousIdentity.anonSessionId,
          device_fingerprint: PORTAL_DEVICE_FINGERPRINT,
          repo_url: parsedRepo.repoUrl,
          branch: branch.trim() || "main",
          commit: null,
        }),
      };
      const analyzeResponse = await fetch(ANALYZE_PATH, analyzeRequest);
      const analyzed = await readJsonResponse<AnalyzeResponse>(analyzeResponse);
      setAnalyzeResult(analyzed);
      setAnalyzedRepoUrl(parsedRepo.repoUrl);

      if (!analyzed.fingerprint_id) {
        return;
      }

      try {
        const intelligenceResponse = await fetch(
          `/api/proxy/api/repositories/${encodeURIComponent(analyzed.fingerprint_id)}/intelligence`,
          {
            method: "GET",
            cache: "no-store",
          },
        );
        const intelligenceBody = await readJsonResponse<RepositoryIntelligenceResponse>(
          intelligenceResponse,
        );
        setIntelligence(intelligenceBody);
      } catch (caught) {
        setError(
          caught instanceof Error
            ? `Analysis succeeded, but repository intelligence could not be loaded: ${caught.message}`
            : "Analysis succeeded, but repository intelligence could not be loaded.",
        );
      }

      try {
        const askResponse = await fetch(
          `/api/proxy/api/repositories/${encodeURIComponent(analyzed.fingerprint_id)}/ask`,
          {
            method: "POST",
            headers: {
              "Content-Type": "application/json",
            },
            body: JSON.stringify({ question: DEFAULT_ASK_QUESTION }),
          },
        );
        const askBody = await readJsonResponse<RepositoryAskResponse>(askResponse);
        setRepoAnswer(askBody);
      } catch (caught) {
        setRepoAnswer(null);
        setError(
          caught instanceof Error
            ? `Analysis succeeded, but repository summary could not be loaded: ${caught.message}`
            : "Analysis succeeded, but repository summary could not be loaded.",
        );
      }
    } catch (caught) {
      setAnalyzeResult(null);
      setAnalyzedRepoUrl(null);
      setIntelligence(null);
      setRepoAnswer(null);
      setError(caught instanceof Error ? caught.message : "Analyze request failed.");
    } finally {
      setAnalyzing(false);
    }
  }

  async function handleRun() {
    if (!parsedRepo) {
      setError("Paste a GitHub repository first.");
      return;
    }

    setError(null);
    setRunning(true);
    try {
      const response = await fetch("/api/proxy/api/v1/executions", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          org_id: null,
          user_id: null,
          anon_user_id: createAnonymousId("anon-portal"),
          anon_session_id: createAnonymousId("portal-session"),
          device_fingerprint: "portal-home",
          repo_url: parsedRepo.repoUrl,
          branch: branch.trim() || "main",
          commit: null,
        }),
      });

      const body = await readJsonResponse<RunResponse>(response);
      setRunResult(body);
    } catch (caught) {
      setRunResult(null);
      setError(caught instanceof Error ? caught.message : "Run request failed.");
    } finally {
      setRunning(false);
    }
  }

  const healthScore = scoreValue(
    intelligence?.repository_identity?.health_score,
    intelligence?.health_score,
  );
  const executionScore = scoreValue(
    intelligence?.repository_identity?.execution_score,
    intelligence?.execution_score,
  );
  const healingScore = scoreValue(
    intelligence?.repository_identity?.healing_score,
    intelligence?.healing_score,
  );
  const repositoryName = parsedRepo
    ? `${parsedRepo.owner}/${parsedRepo.repo}`
    : NO_REPOSITORY_SELECTED;
  const avatarLetter = parsedRepo?.owner?.charAt(0).toUpperCase() || DEFAULT_AVATAR_LETTER;

  return (
    <main className={styles.page}>
      <aside className={styles.sidebar}>
        <div className={styles.profileCard}>
          <div className={styles.avatar}>{avatarLetter}</div>
          <div>
            <strong>{PORTAL_NAME}</strong>
            <p>{repositoryName}</p>
          </div>
        </div>

        <button type="button" onClick={handleAnalyze} disabled={!canAnalyze} className={styles.analyzeButton}>
          {analyzing ? "Analyzing..." : "Analyze Repository"}
        </button>

        <nav className={styles.navSection} aria-label="Portal sections">
          <p className={styles.navHeading}>Workspace</p>
          <ul className={styles.navList}>
            <li className={styles.navItem}>
              <span>Intelligence</span>
              <strong>{analyzeResult ? "1" : "0"}</strong>
            </li>
            <li className={styles.navItem}>
              <span>Executions</span>
              <strong>{runResult ? "1" : "0"}</strong>
            </li>
            <li className={styles.navItem}>
              <span>Errors</span>
              <strong>{error ? "1" : "0"}</strong>
            </li>
          </ul>
        </nav>

        <div className={styles.sidebarMeta}>
          <p>
            Branch <strong>{branch.trim() || "main"}</strong>
          </p>
          <p>
            API <code>{API_BASE_URL}</code>
          </p>
        </div>
      </aside>

      <section className={styles.listPane}>
        <header className={styles.paneHeader}>
          <h1>Repository workspace</h1>
          <p>Paste a GitHub URL, analyze metadata, and prepare execution.</p>
        </header>

        <section className={styles.panel}>
          <h2>Repository input</h2>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              handleAnalyze();
            }}
          >
            <label htmlFor="github-repo-url" className={styles.label}>
              GitHub repository URL or owner/repo
            </label>
            <input
              id="github-repo-url"
              type="text"
              autoComplete="off"
              value={repository}
              onChange={(event) => resetResults(event.target.value)}
              onPaste={(event) => {
                const pasted = event.clipboardData.getData("text");
                if (pasted) {
                  event.preventDefault();
                  resetResults(pasted.trim());
                }
              }}
              placeholder="https://github.com/owner/repo"
              className={styles.input}
            />
            <p className={styles.hint}>
              You can also use <code>owner/repo</code>.
            </p>

            <div className={styles.branchRow}>
              <label htmlFor="branch" className={styles.label}>
                Branch
              </label>
              <input
                id="branch"
                type="text"
                value={branch}
                onChange={(event) => setBranch(event.target.value)}
                placeholder="main"
                className={styles.input}
              />
            </div>

            <div className={styles.actions}>
              <button type="submit" disabled={!canAnalyze} className={styles.primaryButton}>
                {analyzing ? "Analyzing repository..." : "Analyze and get intelligence"}
              </button>
              <button type="button" onClick={handleRun} disabled={!canRun} className={styles.secondaryButton}>
                {running ? "Starting run..." : "Run repository"}
              </button>
            </div>
          </form>
        </section>

        {error ? (
          <section className={styles.errorPanel} role="alert">
            {error}
          </section>
        ) : null}

        {analyzeResult ? (
          <section className={styles.panel}>
            <h2>Intelligence</h2>
            <div className={styles.grid}>
              <div className={styles.tile}>
                <strong>Repository</strong>
                <code>{analyzeResult.repo_url ?? "n/a"}</code>
              </div>
              <div className={styles.tile}>
                <strong>Fingerprint</strong>
                <code>{analyzeResult.fingerprint_id ?? "pending"}</code>
              </div>
              <div className={styles.tile}>
                <strong>Frameworks</strong>
                <span>{(analyzeResult.frameworks ?? []).join(", ") || "n/a"}</span>
              </div>
              <div className={styles.tile}>
                <strong>Services</strong>
                <span>{(analyzeResult.services ?? []).join(", ") || "n/a"}</span>
              </div>
              <div className={styles.tile}>
                <strong>Health score</strong>
                <span>{formatScore(healthScore)}</span>
              </div>
              <div className={styles.tile}>
                <strong>Execution score</strong>
                <span>{formatScore(executionScore)}</span>
              </div>
              <div className={styles.tile}>
                <strong>Healing score</strong>
                <span>{formatScore(healingScore)}</span>
              </div>
              <div className={styles.tile}>
                <strong>Runtime</strong>
                <span>{intelligence?.runtime ?? "n/a"}</span>
              </div>
            </div>

            {repoAnswer?.answer ? (
              <div className={styles.answerBox}>
                <h3>Repo summary</h3>
                <p>{repoAnswer.answer}</p>
                <p className={styles.hint}>
                  Confidence: {formatConfidence(repoAnswer.confidence)}
                </p>
              </div>
            ) : null}
          </section>
        ) : (
          <section className={styles.emptyPanel}>
            <h2>{EMPTY_STATE_HEADING}</h2>
            <p>Analyze a repository to view frameworks, services, and score details.</p>
          </section>
        )}
      </section>

      <section className={styles.detailPane}>
        <header className={styles.paneHeader}>
          <h2>Execution details</h2>
          <p>Launch and monitor execution from the same workspace.</p>
        </header>

        {workspace ? (
          <section className={styles.panel}>
            <div className={styles.workspaceHeader}>
              <h2>Run status</h2>
              <span className={`${styles.badge} ${
                workspace.state === "Running" ? styles.badgeRunning :
                workspace.state === "Failed" ? styles.badgeFailed :
                workspace.state === "Stopped" || workspace.state === "Destroyed" ? styles.badgeStopped :
                styles.badgeStarting
              }`}>
                {ACTIVE_WORKSPACE_STATES.has(workspace.state) && <span className={styles.dot} />}
                {workspace.state}
              </span>
            </div>
            <div className={styles.grid}>
              <div className={styles.tile}>
                <strong>Execution ID</strong>
                <code>{runResult?.execution_id ?? "n/a"}</code>
              </div>
              <div className={styles.tile}>
                <strong>Framework</strong>
                <span>{workspace.framework}</span>
              </div>
              <div className={styles.tile}>
                <strong>Memory</strong>
                <span>{workspace.resource_quotas.max_memory_mb} MB</span>
              </div>
              <div className={styles.tile}>
                <strong>CPU</strong>
                <span>{workspace.resource_quotas.max_cpu_millis} m</span>
              </div>
              {workspace.ports.map((p, i) => (
                <div key={i} className={styles.tile}>
                  <strong>Port {p.port}</strong>
                  <a
                    href={`/api/app-proxy/${workspace.id}${p.route || "/"}`}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Open app ↗
                  </a>
                </div>
              ))}
            </div>

            <div className={styles.logSection}>
              <div className={styles.logHeader}>
                <span className={styles.logTitle}>Logs</span>
                {ACTIVE_WORKSPACE_STATES.has(workspace.state) && (
                  <span className={styles.livePill}>
                    <span className={styles.dot} /> live
                  </span>
                )}
              </div>
              <div className={styles.logBox} ref={logBoxRef}>
                {workspaceLogs.length === 0 ? (
                  <span className={styles.logEmpty}>No logs yet…</span>
                ) : (
                  workspaceLogs.map((line, i) => (
                    <div key={i} className={styles.logLine}>
                      <span className={styles.logNum}>{i + 1}</span>
                      <span>{line}</span>
                    </div>
                  ))
                )}
              </div>
            </div>
          </section>
        ) : runResult ? (
          <section className={styles.panel}>
            <h2>Run status</h2>
            <div className={styles.grid}>
              <div className={styles.tile}>
                <strong>Execution ID</strong>
                <code>{runResult.execution_id ?? "n/a"}</code>
              </div>
              <div className={styles.tile}>
                <strong>Status</strong>
                <span>{runResult.status ?? "starting"}</span>
              </div>
            </div>
          </section>
        ) : (
          <section className={styles.emptyPanel}>
            <h2>{EMPTY_STATE_HEADING}</h2>
            <p>Run a repository to populate execution status and workspace links.</p>
          </section>
        )}

        <section className={styles.footerInfo}>
          <p>
            API base: <code>{API_BASE_URL}</code>
          </p>
          <p>
            Health check: <a href="/api/health">/api/health</a>
          </p>
        </section>
      </section>
    </main>
  );
}

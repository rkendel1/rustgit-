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

type LaunchOverridesPayload = {
  branch?: string;
  start_command?: string;
  environment?: Record<string, string>;
  versions?: Record<string, string>;
};

type WorkspaceFilesResponse = {
  files?: string[];
};

type WorkspaceFileResponse = {
  path?: string;
  content?: string;
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
  resource_quotas?: { max_memory_mb: number; max_cpu_millis: number };
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

function parseKeyValueLines(input: string): Record<string, string> {
  return input
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .reduce<Record<string, string>>((acc, line) => {
      const separatorIndex = line.indexOf("=");
      if (separatorIndex < 0) return acc;
      const key = line.slice(0, separatorIndex).trim();
      const value = line.slice(separatorIndex + 1).trim();
      if (!key) return acc;
      acc[key] = value;
      return acc;
    }, {});
}

function encodeWorkspacePath(path: string): string {
  return path
    .split("/")
    .filter(Boolean)
    .map((segment) => encodeURIComponent(segment))
    .join("/");
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
  const repoInputRef = useRef<HTMLInputElement>(null);
  const [repository, setRepository] = useState("");
  const [branch, setBranch] = useState("main");
  const [startCommand, setStartCommand] = useState("");
  const [envOverrides, setEnvOverrides] = useState("");
  const [versionOverrides, setVersionOverrides] = useState("");
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
  const [workspaceFiles, setWorkspaceFiles] = useState<string[]>([]);
  const [selectedWorkspaceFile, setSelectedWorkspaceFile] = useState<string | null>(null);
  const [selectedWorkspaceFileContent, setSelectedWorkspaceFileContent] = useState("");
  const [workspaceFilesLoading, setWorkspaceFilesLoading] = useState(false);
  const [workspaceFilesError, setWorkspaceFilesError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const [freeingSpace, setFreeingSpace] = useState(false);
  const [freeSpaceResult, setFreeSpaceResult] = useState<string | null>(null);
  const logBoxRef = useRef<HTMLDivElement>(null);
  const anonymousIdentity = useMemo(
    () => ({
      anonUserId: createAnonymousId("anon-portal"),
      anonSessionId: createAnonymousId("portal-session"),
    }),
    [],
  );

  // Restore last URL from localStorage after hydration (safe — runs client-only)
  useEffect(() => {
    try {
      const saved = localStorage.getItem("rustgit:lastRepoUrl");
      if (saved) setRepository(saved);
    } catch { /* ignore */ }
  }, []);

  const parsedRepo = useMemo(() => parseRepositoryInput(repository), [repository]);
  const canAnalyze = Boolean(parsedRepo) && !analyzing;
  const canRun = Boolean(parsedRepo) && !running;

  function resetResults(nextRepositoryValue: string) {
    setRepository(nextRepositoryValue);
    try { localStorage.setItem("rustgit:lastRepoUrl", nextRepositoryValue); } catch { /* ignore */ }
    setAnalyzeResult(null);
    setAnalyzedRepoUrl(null);
    setIntelligence(null);
    setRepoAnswer(null);
    setRunResult(null);
    setWorkspace(null);
    setWorkspaceLogs([]);
    setWorkspaceFiles([]);
    setSelectedWorkspaceFile(null);
    setSelectedWorkspaceFileContent("");
    setWorkspaceFilesError(null);
    setError(null);
  }

  function buildLaunchOverrides(): LaunchOverridesPayload {
    const payload: LaunchOverridesPayload = {};
    const b = branch.trim();
    const command = startCommand.trim();
    const environment = parseKeyValueLines(envOverrides);
    const versions = parseKeyValueLines(versionOverrides);
    if (b) payload.branch = b;
    if (command) payload.start_command = command;
    if (Object.keys(environment).length > 0) payload.environment = environment;
    if (Object.keys(versions).length > 0) payload.versions = versions;
    return payload;
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

  async function handleStop() {
    const wsId = runResult?.execution_id;
    if (!wsId) return;
    setActionPending(true);
    try {
      await fetch(`/api/proxy/workspaces/${wsId}`, { method: "DELETE" });
      await fetchWorkspaceData(wsId);
    } finally {
      setActionPending(false);
    }
  }

  async function handleRestart() {
    const wsId = runResult?.execution_id;
    if (!wsId) return;
    setActionPending(true);
    try {
      await fetch(`/api/proxy/workspaces/${wsId}/restart`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(buildLaunchOverrides()),
      });
      // Re-enable polling by touching state; the existing poll effect picks it up
      await fetchWorkspaceData(wsId);
    } finally {
      setActionPending(false);
    }
  }

  // Poll workspace while it's active (or after restart kicks it back to active)
  useEffect(() => {
    const wsId = runResult?.execution_id;
    if (!wsId) return;
    // If we already have a terminal state and no pending action, skip re-polling
    if (workspace && !ACTIVE_WORKSPACE_STATES.has(workspace.state) && !actionPending) return;
    let cancelled = false;

    async function poll() {
      try {
        const ws = await fetchWorkspaceData(wsId!);
        if (cancelled) return;
        if (ACTIVE_WORKSPACE_STATES.has(ws.state)) {
          setTimeout(poll, 3000);
        }
      } catch {
        if (!cancelled) setTimeout(poll, 3000);
      }
    }

    poll();
    return () => { cancelled = true; };
  }, [runResult?.execution_id, fetchWorkspaceData, actionPending]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const wsId = runResult?.execution_id;
    if (!wsId) return;
    let cancelled = false;

    async function loadFiles() {
      setWorkspaceFilesLoading(true);
      try {
        const response = await fetch(`/api/proxy/workspaces/${wsId}/files`, { cache: "no-store" });
        const payload = await readJsonResponse<WorkspaceFilesResponse>(response);
        if (cancelled) return;
        const files = (payload.files ?? []).slice(0, 200);
        setWorkspaceFiles(files);
        if (files.length === 0) {
          setSelectedWorkspaceFile(null);
          setSelectedWorkspaceFileContent("");
        } else if (!selectedWorkspaceFile || !files.includes(selectedWorkspaceFile)) {
          setSelectedWorkspaceFile(files[0]);
        }
        setWorkspaceFilesError(null);
      } catch (caught) {
        if (cancelled) return;
        setWorkspaceFilesError(caught instanceof Error ? caught.message : "Failed to load workspace files.");
      } finally {
        if (!cancelled) setWorkspaceFilesLoading(false);
      }
    }

    loadFiles();
    return () => { cancelled = true; };
  }, [runResult?.execution_id, actionPending]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const wsId = runResult?.execution_id;
    if (!wsId || !selectedWorkspaceFile) return;
    const selectedFile = selectedWorkspaceFile;
    let cancelled = false;

    async function loadFileContent() {
      try {
        const response = await fetch(
          `/api/proxy/workspaces/${wsId}/files/${encodeWorkspacePath(selectedFile)}`,
          { cache: "no-store" },
        );
        const payload = await readJsonResponse<WorkspaceFileResponse>(response);
        if (!cancelled) {
          setSelectedWorkspaceFileContent(payload.content ?? "");
        }
      } catch {
        if (!cancelled) {
          setSelectedWorkspaceFileContent("");
        }
      }
    }

    loadFileContent();
    return () => { cancelled = true; };
  }, [runResult?.execution_id, selectedWorkspaceFile]);

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
    // If autofill filled the DOM but skipped React state, sync now
    const domValue = repoInputRef.current?.value ?? "";
    if (domValue && domValue !== repository) resetResults(domValue);
    const repo = parsedRepo ?? parseRepositoryInput(domValue);
    if (!repo) {
      setError("Enter a valid GitHub URL — e.g. https://github.com/owner/repo");
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
          repo_url: repo.repoUrl,
          branch: branch.trim() || "main",
          commit: null,
        }),
      };
      const analyzeResponse = await fetch(ANALYZE_PATH, analyzeRequest);
      const analyzed = await readJsonResponse<AnalyzeResponse>(analyzeResponse);
      setAnalyzeResult(analyzed);
      setAnalyzedRepoUrl(repo.repoUrl);

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
    const domValue = repoInputRef.current?.value ?? "";
    if (domValue && domValue !== repository) resetResults(domValue);
    const repo = parsedRepo ?? parseRepositoryInput(domValue);
    if (!repo) {
      setError("Enter a valid GitHub URL — e.g. https://github.com/owner/repo");
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
          repo_url: repo.repoUrl,
          branch: branch.trim() || "main",
          commit: null,
          ...buildLaunchOverrides(),
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

  async function handleFreeSpace() {
    setFreeingSpace(true);
    setFreeSpaceResult(null);
    try {
      const response = await fetch("/api/proxy/api/cleanup", { method: "POST" });
      if (response.ok) {
        const data = await response.json() as { evicted_workspaces: number; free_gb: number };
        const gb = data.free_gb?.toFixed(1) ?? "?";
        setFreeSpaceResult(`Freed ${data.evicted_workspaces} workspace(s). ${gb} GB now available.`);
      } else {
        setFreeSpaceResult("Cleanup request failed.");
      }
    } catch {
      setFreeSpaceResult("Cleanup request failed.");
    } finally {
      setFreeingSpace(false);
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

        <div className={styles.sidebarInput}>
          <label htmlFor="github-repo-url" className={styles.label}>
            Repository
          </label>
          <input
            ref={repoInputRef}
            id="github-repo-url"
            type="text"
            autoComplete="off"
            value={repository}
            onChange={(event) => resetResults(event.target.value)}
            onInput={(event) => {
              const val = (event.target as HTMLInputElement).value;
              if (val !== repository) resetResults(val);
            }}
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
          <p className={styles.hint}>Paste a GitHub URL or <code>owner/repo</code>.</p>
          <button type="button" onClick={handleAnalyze} disabled={analyzing} className={styles.analyzeButton}>
            {analyzing ? "Analyzing..." : "Analyze"}
          </button>
          <button type="button" onClick={handleRun} disabled={running} className={styles.secondaryButton}>
            {running ? "Starting..." : "Run"}
          </button>
        </div>

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

        <button type="button" onClick={handleFreeSpace} disabled={freeingSpace} className={styles.freeSpaceButton}>
          {freeingSpace ? "Cleaning..." : "Free Space"}
        </button>
        {freeSpaceResult ? (
          <p className={styles.freeSpaceResult}>{freeSpaceResult}</p>
        ) : null}
      </aside>

      <section className={styles.listPane}>
        <header className={styles.paneHeader}>
          <h1>Intelligence</h1>
          <p>Analysis results for the selected repository.</p>
        </header>

        {error ? (
          <section className={styles.errorPanel} role="alert">
            {error}
          </section>
        ) : null}

        {analyzeResult ? (
          <section className={styles.panel}>
            <h2>Analysis</h2>
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

        {runResult?.execution_id ? (
          <section className={styles.panel}>
            <h2>Workspace files (pre-heal)</h2>
            <p className={styles.hint}>
              Inspect files to decide which environment variables, versions, and start commands to set.
            </p>
            {workspaceFilesError ? <p className={styles.hint}>{workspaceFilesError}</p> : null}
            {workspaceFilesLoading ? (
              <p className={styles.hint}>Loading files…</p>
            ) : workspaceFiles.length === 0 ? (
              <p className={styles.hint}>No files available yet.</p>
            ) : (
              <>
                <div className={styles.actions}>
                  {workspaceFiles.slice(0, 24).map((file) => (
                    <button
                      key={file}
                      type="button"
                      className={styles.btn}
                      onClick={() => setSelectedWorkspaceFile(file)}
                      disabled={selectedWorkspaceFile === file}
                    >
                      {file}
                    </button>
                  ))}
                </div>
                <div className={styles.logSection}>
                  <div className={styles.logHeader}>
                    <span className={styles.logTitle}>{selectedWorkspaceFile ?? "Select a file"}</span>
                  </div>
                  <div className={styles.logBox}>
                    <pre>{selectedWorkspaceFileContent || "No content available."}</pre>
                  </div>
                </div>
              </>
            )}
          </section>
        ) : null}

        {workspace ? (
          <section className={styles.panel}>
            <div className={styles.workspaceHeader}>
              <h2>Run status</h2>
              <div style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
                <span className={`${styles.badge} ${
                  workspace.state === "Running" ? styles.badgeRunning :
                  workspace.state === "Failed" ? styles.badgeFailed :
                  workspace.state === "Stopped" || workspace.state === "Destroyed" ? styles.badgeStopped :
                  styles.badgeStarting
                }`}>
                  {ACTIVE_WORKSPACE_STATES.has(workspace.state) && <span className={styles.dot} />}
                  {workspace.state}
                </span>
                <button
                  className={styles.btnRestart}
                  disabled={
                    actionPending ||
                    ["Created", "Materializing", "Analyzing", "Planning",
                     "Provisioning", "Starting", "Restarting", "Stopping",
                     "Running", "Destroyed"].includes(workspace.state)
                  }
                  onClick={handleRestart}
                >
                  Retry run
                </button>
                <button
                  className={styles.btnStop}
                  disabled={actionPending || ["Stopped","Destroyed","Stopping"].includes(workspace.state)}
                  onClick={handleStop}
                >
                  Stop
                </button>
              </div>
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
                <span>{workspace.resource_quotas?.max_memory_mb ?? "—"} MB</span>
              </div>
              <div className={styles.tile}>
                <strong>CPU</strong>
                <span>{workspace.resource_quotas?.max_cpu_millis ?? "—"} m</span>
              </div>
              {(workspace.ports ?? []).map((p, i) => (
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

            {workspace.state === "Failed" && (
              <div className={styles.healPanel}>
                <p className={styles.healTitle}>Heal &amp; retry</p>
                <label htmlFor="heal-branch" className={styles.label}>Branch</label>
                <input
                  id="heal-branch"
                  type="text"
                  value={branch}
                  onChange={(e) => setBranch(e.target.value)}
                  placeholder="main"
                  className={styles.input}
                />
                <label htmlFor="heal-cmd" className={styles.label}>Start command override</label>
                <input
                  id="heal-cmd"
                  type="text"
                  value={startCommand}
                  onChange={(e) => setStartCommand(e.target.value)}
                  placeholder="npm run dev -- --host 0.0.0.0"
                  className={styles.input}
                />
                <label htmlFor="heal-env" className={styles.label}>Environment overrides (KEY=value per line)</label>
                <textarea
                  id="heal-env"
                  value={envOverrides}
                  onChange={(e) => setEnvOverrides(e.target.value)}
                  placeholder={"PORT=3000\nNODE_ENV=development"}
                  className={styles.input}
                  rows={3}
                />
                <label htmlFor="heal-versions" className={styles.label}>Version overrides (KEY=value per line)</label>
                <textarea
                  id="heal-versions"
                  value={versionOverrides}
                  onChange={(e) => setVersionOverrides(e.target.value)}
                  placeholder={"NODE_VERSION=20\nPYTHON_VERSION=3.12"}
                  className={styles.input}
                  rows={2}
                />
              </div>
            )}

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
            <a href="/api/health">/api/health</a>
          </p>
        </section>
      </section>
    </main>
  );
}

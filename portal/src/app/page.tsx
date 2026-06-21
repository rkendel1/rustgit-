"use client";

import { useMemo, useState } from "react";
import styles from "./page.module.css";

const API_BASE_URL =
  process.env.NEXT_PUBLIC_API_URL ??
  (process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : "https://api.trythissoftware.com");

const DEFAULT_ASK_QUESTION = "Summarize what this repository does and the best way to run it.";
const SCORE_DECIMAL_PLACES = 1;
const CONFIDENCE_DECIMAL_PLACES = 2;

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
      return {
        owner,
        repo,
        repoUrl: `https://github.com/${owner}/${repo}`,
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
    repoUrl: `https://github.com/${owner}/${repo}`,
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
    setError(null);
  }

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
        body: JSON.stringify({ repo_url: parsedRepo.repoUrl }),
      };
      let analyzeResponse: Response;
      try {
        const analyzeV1Response = await fetch("/api/proxy/api/v1/repositories/analyze", analyzeRequest);
        analyzeResponse =
          analyzeV1Response.status === 404
            ? await fetch("/api/proxy/api/repositories/analyze", analyzeRequest)
            : analyzeV1Response;
      } catch {
        analyzeResponse = await fetch("/api/proxy/api/repositories/analyze", analyzeRequest);
      }
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

  return (
    <main className={styles.page}>
      <section className={styles.hero}>
        <h1>Paste a GitHub repo. Get intelligence. Run it.</h1>
        <p>
          Start with a GitHub link, get an instant readout of frameworks/services and a run summary,
          then launch execution.
        </p>
      </section>

      <section className={styles.panel}>
        <h2>1) Repository input</h2>
        <label htmlFor="github-repo-url" className={styles.label}>
          GitHub repository URL or owner/repo
        </label>
        <input
          id="github-repo-url"
          value={repository}
          onChange={(event) => resetResults(event.target.value)}
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
            value={branch}
            onChange={(event) => setBranch(event.target.value)}
            placeholder="main"
            className={styles.input}
          />
        </div>

        <div className={styles.actions}>
          <button type="button" onClick={handleAnalyze} disabled={!canAnalyze} className={styles.primaryButton}>
            {analyzing ? "Analyzing repository..." : "Analyze and get intelligence"}
          </button>
          <button type="button" onClick={handleRun} disabled={!canRun} className={styles.secondaryButton}>
            {running ? "Starting run..." : "Run repository"}
          </button>
        </div>
      </section>

      {error ? (
        <section className={styles.errorPanel} role="alert">
          {error}
        </section>
      ) : null}

      {analyzeResult ? (
        <section className={styles.panel}>
          <h2>2) Intelligence</h2>
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
      ) : null}

      {runResult ? (
        <section className={styles.panel}>
          <h2>3) Run status</h2>
          <div className={styles.grid}>
            <div className={styles.tile}>
              <strong>Execution ID</strong>
              <code>{runResult.execution_id ?? "n/a"}</code>
            </div>
            <div className={styles.tile}>
              <strong>Workspace ID</strong>
              <code>{runResult.workspace_id ?? "n/a"}</code>
            </div>
            <div className={styles.tile}>
              <strong>Status</strong>
              <span>{runResult.status ?? "starting"}</span>
            </div>
            <div className={styles.tile}>
              <strong>Workspace URL</strong>
              {runResult.workspace_url ? (
                <a
                  href={runResult.workspace_url}
                  target="_blank"
                  rel="noreferrer"
                  aria-label="Open workspace in a new tab"
                >
                  Open workspace
                </a>
              ) : (
                <span>pending</span>
              )}
            </div>
          </div>
        </section>
      ) : null}

      <section className={styles.footerInfo}>
        <p>
          API base: <code>{API_BASE_URL}</code>
        </p>
        <p>
          Health check: <a href="/api/health">/api/health</a>
        </p>
      </section>
    </main>
  );
}

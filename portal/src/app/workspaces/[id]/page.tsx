"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import styles from "./page.module.css";

const API_BASE =
  process.env.NEXT_PUBLIC_API_URL ??
  (process.env.NODE_ENV === "development"
    ? "http://localhost:8080"
    : "https://api.trythissoftware.com");

type WorkspaceState =
  | "Created" | "Materializing" | "Analyzing" | "Planning" | "Pending"
  | "Provisioning" | "Starting" | "Running" | "Degraded" | "Restarting"
  | "Migrating" | "Paused" | "Failed" | "Stopping" | "Stopped" | "Destroyed";

interface Workspace {
  id: string;
  repo_url: string;
  state: WorkspaceState;
  framework: string;
  ports: { port: number; protocol: string; route: string }[];
  network_policy: { allow_outbound: boolean; allowed_hosts: string[] };
  resource_quotas?: { max_memory_mb: number; max_cpu_millis: number };
}

const ACTIVE_STATES = new Set<WorkspaceState>([
  "Created", "Materializing", "Analyzing", "Planning",
  "Provisioning", "Starting", "Running", "Restarting",
]);

function badgeClass(state: WorkspaceState): string {
  if (state === "Running") return styles.running;
  if (state === "Failed") return styles.failed;
  if (state === "Stopped" || state === "Destroyed") return styles.stopped;
  if (ACTIVE_STATES.has(state)) return styles.starting;
  return styles.other;
}

export default function WorkspacePage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const [id, setId] = useState<string | null>(null);
  const [workspace, setWorkspace] = useState<Workspace | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [actionPending, setActionPending] = useState(false);
  const logBoxRef = useRef<HTMLDivElement>(null);

  // Resolve the async params once on mount
  useEffect(() => {
    params.then((p) => setId(p.id));
  }, [params]);

  const fetchWorkspace = useCallback(async (workspaceId: string) => {
    const res = await fetch(`${API_BASE}/workspaces/${workspaceId}`, {
      cache: "no-store",
    });
    if (!res.ok) throw new Error(`Workspace not found (${res.status})`);
    return res.json() as Promise<Workspace>;
  }, []);

  const fetchLogs = useCallback(async (workspaceId: string) => {
    const res = await fetch(`${API_BASE}/workspaces/${workspaceId}/logs`, {
      cache: "no-store",
    });
    if (!res.ok) return [];
    const data: { logs: string[] } = await res.json();
    return data.logs;
  }, []);

  // Initial load + polling
  useEffect(() => {
    if (!id) return;
    let cancelled = false;

    async function load() {
      try {
        const [ws, logLines] = await Promise.all([
          fetchWorkspace(id!),
          fetchLogs(id!),
        ]);
        if (cancelled) return;
        setWorkspace(ws);
        setLogs(logLines);
        setError(null);
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : "Failed to load workspace");
      }
    }

    load();
    const interval = setInterval(() => {
      load();
    }, 3000);

    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, [id, fetchWorkspace, fetchLogs]);

  // Auto-scroll logs
  useEffect(() => {
    if (logBoxRef.current) {
      logBoxRef.current.scrollTop = logBoxRef.current.scrollHeight;
    }
  }, [logs]);

  async function handleAction(method: string, path: string) {
    if (!id) return;
    setActionPending(true);
    try {
      await fetch(`${API_BASE}/workspaces/${id}${path}`, { method });
    } finally {
      setActionPending(false);
    }
  }

  const isActive = workspace ? ACTIVE_STATES.has(workspace.state) : false;
  const canStop = workspace
    ? ["Running", "Degraded", "Paused", "Starting"].includes(workspace.state)
    : false;
  const canRestart = workspace
    ? ["Running", "Stopped", "Failed", "Degraded"].includes(workspace.state)
    : false;

  if (error) {
    return (
      <div className={styles.page}>
        <div className={styles.errorBox}>{error}</div>
      </div>
    );
  }

  if (!workspace) {
    return <div className={styles.page}><p className={styles.loading}>Loading…</p></div>;
  }

  const repoShort = workspace.repo_url.replace(/^https?:\/\/github\.com\//, "").replace(/\.git$/, "");

  return (
    <div className={styles.page}>
      <div className={styles.header}>
        <div>
          <p className={styles.breadcrumb}>
            <a href="/">Portal</a> / workspace
          </p>
          <p className={styles.title}>{repoShort}</p>
        </div>
        <div style={{ display: "flex", alignItems: "center", gap: "0.75rem", flexWrap: "wrap" }}>
          <span className={`${styles.badge} ${badgeClass(workspace.state)}`}>
            {isActive && <span className={styles.dot} />}
            {workspace.state}
          </span>
          <div className={styles.actions}>
            <button
              className={styles.btn}
              disabled={!canRestart || actionPending}
              onClick={() => handleAction("POST", "/restart")}
            >
              Restart
            </button>
            <button
              className={`${styles.btn} ${styles.btnDanger}`}
              disabled={!canStop || actionPending}
              onClick={() => handleAction("DELETE", "")}
            >
              Stop
            </button>
          </div>
        </div>
      </div>

      <div className={styles.grid}>
        <div className={styles.tile}>
          <strong>Workspace ID</strong>
          <code>{workspace.id}</code>
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
        {workspace.ports.map((p, i) => {
          const proxyUrl = `/api/app-proxy/${workspace.id}${p.route || "/"}`;
          return (
            <div key={i} className={styles.tile}>
              <strong>Port {p.port}</strong>
              <a href={proxyUrl} target="_blank" rel="noopener noreferrer">
                Open app ↗
              </a>
            </div>
          );
        })}
      </div>

      <div className={styles.section}>
        <div className={styles.sectionHeader}>
          <span className={styles.sectionTitle}>Logs</span>
          {isActive && (
            <span className={styles.liveDot}>
              <span className={styles.dot} /> live
            </span>
          )}
        </div>
        <div className={styles.logBox} ref={logBoxRef}>
          {logs.length === 0 ? (
            <span className={styles.empty}>No logs yet.</span>
          ) : (
            logs.map((line, i) => (
              <div key={i} className={styles.logLine}>
                <span className={styles.logNum}>{i + 1}</span>
                <span>{line}</span>
              </div>
            ))
          )}
        </div>
      </div>
    </div>
  );
}

/**
 * Typed API client for the defi-flow Rust backend.
 */

const API_BASE = process.env.NEXT_PUBLIC_API_URL || "";

async function request<T>(
  path: string,
  options?: RequestInit
): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    headers: { "Content-Type": "application/json", ...options?.headers },
    ...options,
  });

  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error || `API error ${res.status}`);
  }

  return res.json();
}

// ── Validate ──────────────────────────────────────────────────────────

export type ValidateResponse = {
  valid: boolean;
  errors?: string[];
  warnings?: string[];
};

export async function validateWorkflow(
  workflow: unknown,
  checkOnchain = false
): Promise<ValidateResponse> {
  return request<ValidateResponse>("/api/validate", {
    method: "POST",
    body: JSON.stringify({ workflow, check_onchain: checkOnchain }),
  });
}

// ── Backtest ──────────────────────────────────────────────────────────

export type TrajectoryPoint = { timestamp: number; tvl: number };

export type BacktestResult = {
  label: string;
  twrr_pct: number;
  annualized_pct: number;
  max_drawdown_pct: number;
  sharpe: number;
  net_pnl: number;
  rebalances: number;
  liquidations: number;
  funding_pnl: number;
  rewards_pnl: number;
  premium_pnl: number;
  lp_fees: number;
  lending_interest: number;
  swap_costs: number;
  ticks: number;
  trajectory: TrajectoryPoint[];
};

export type MonteCarloOutput = {
  n_simulations: number;
  simulations: BacktestResult[];
};

export type BacktestResponse = {
  id: string;
  result: BacktestResult;
  monte_carlo?: MonteCarloOutput;
};

export type BacktestSummary = {
  id: string;
  label: string;
  twrr_pct: number;
  sharpe: number;
  max_drawdown_pct: number;
  created_at: number;
};

export async function runBacktest(
  workflow: unknown,
  options: {
    capital?: number;
    slippage_bps?: number;
    seed?: number;
    data_dir?: string;
    auto_fetch?: boolean;
    monte_carlo?: number;
  } = {}
): Promise<BacktestResponse> {
  return request<BacktestResponse>("/api/backtest", {
    method: "POST",
    body: JSON.stringify({ workflow, ...options }),
  });
}

export async function listBacktests(): Promise<BacktestSummary[]> {
  return request<BacktestSummary[]>("/api/backtests");
}

export async function getBacktest(id: string) {
  return request<{
    id: string;
    workflow: unknown;
    result: BacktestResult;
    created_at: number;
  }>(`/api/backtest/${id}`);
}

// ── Run (daemon) ──────────────────────────────────────────────────────

export type RunStartResponse = {
  session_id: string;
  status: string;
};

export type RunStatusResponse = {
  session_id: string;
  status: string;
  tvl: number;
  started_at: number;
  network: string;
  dry_run: boolean;
  workflow_name: string;
};

export type RunListEntry = {
  session_id: string;
  workflow_name: string;
  status: string;
  network: string;
  started_at: number;
};

export async function startDaemon(
  workflow: unknown,
  options: {
    network?: string;
    dry_run?: boolean;
    slippage_bps?: number;
    private_key?: string;
  } = {}
): Promise<RunStartResponse> {
  return request<RunStartResponse>("/api/run/start", {
    method: "POST",
    body: JSON.stringify({ workflow, ...options }),
  });
}

export async function listRuns(): Promise<RunListEntry[]> {
  return request<RunListEntry[]>("/api/runs");
}

export async function getRunStatus(
  sessionId: string
): Promise<RunStatusResponse> {
  return request<RunStatusResponse>(`/api/run/${sessionId}/status`);
}

export async function stopDaemon(sessionId: string) {
  return request<{ session_id: string; status: string }>(
    `/api/run/${sessionId}/stop`,
    { method: "POST" }
  );
}

export function subscribeEvents(sessionId: string): EventSource {
  return new EventSource(`${API_BASE}/api/run/${sessionId}/events`);
}

// ── Data ──────────────────────────────────────────────────────────────

export async function fetchData(
  workflow: unknown,
  options: { days?: number; interval?: string; output_dir?: string } = {}
): Promise<{ status: string; data_dir: string }> {
  return request<{ status: string; data_dir: string }>("/api/data/fetch", {
    method: "POST",
    body: JSON.stringify({ workflow, ...options }),
  });
}

export async function uploadData(files: FileList) {
  const form = new FormData();
  for (const file of files) {
    form.append("file", file);
  }

  const res = await fetch(`${API_BASE}/api/data/upload`, {
    method: "POST",
    body: form,
  });

  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error || `Upload failed: ${res.status}`);
  }

  return res.json();
}

export type DataManifest = {
  files: { name: string; size: number }[];
};

export async function getDataManifest(): Promise<DataManifest> {
  return request<DataManifest>("/api/data/manifest");
}

// ── Schema ────────────────────────────────────────────────────────────

export async function getSchema(): Promise<unknown> {
  return request<unknown>("/api/schema");
}

// ── Health ────────────────────────────────────────────────────────────

export async function checkHealth(): Promise<boolean> {
  try {
    const res = await fetch(`${API_BASE}/health`);
    return res.ok;
  } catch {
    return false;
  }
}

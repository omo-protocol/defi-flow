import type { WalletInfo, StrategyInfo } from "./auth-store";

async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "Content-Type": "application/json", ...init?.headers },
  });
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || "Request failed");
  return data as T;
}

// Auth (register only — login/logout handled by NextAuth)
export const register = (username: string, password: string) =>
  api<{ ok: boolean; username: string }>("/api/auth/register", {
    method: "POST",
    body: JSON.stringify({ username, password }),
  });

// Wallets
export const listWallets = () => api<WalletInfo[]>("/api/auth/wallets");

export const createWallet = (label: string, mode: "generate" | "import", privateKey?: string) =>
  api<WalletInfo & { mnemonic?: string }>("/api/auth/wallets", {
    method: "POST",
    body: JSON.stringify({ label, mode, privateKey }),
  });

export const deleteWallet = (id: string) =>
  api<{ ok: boolean }>(`/api/auth/wallets/${id}`, { method: "DELETE" });

// Strategies
export const listStrategies = () => api<StrategyInfo[]>("/api/auth/strategies");

export const getStrategy = (id: string) =>
  api<StrategyInfo & { workflow_json: string }>(`/api/auth/strategies/${id}`);

export const saveStrategy = (
  name: string,
  workflow_json: unknown,
  wallet_id?: string | null
) =>
  api<{ id: string }>("/api/auth/strategies", {
    method: "POST",
    body: JSON.stringify({ name, workflow_json, wallet_id }),
  });

export const updateStrategy = (
  id: string,
  updates: { name?: string; workflow_json?: unknown; wallet_id?: string | null }
) =>
  api<{ ok: boolean }>(`/api/auth/strategies/${id}`, {
    method: "PUT",
    body: JSON.stringify(updates),
  });

export const deleteStrategy = (id: string) =>
  api<{ ok: boolean }>(`/api/auth/strategies/${id}`, { method: "DELETE" });

// Config
export const getConfig = () => api<Record<string, string>>("/api/auth/config");

export const updateConfig = (updates: Record<string, string | null>) =>
  api<{ ok: boolean }>("/api/auth/config", {
    method: "PUT",
    body: JSON.stringify(updates),
  });

// Run
export const startRun = (wallet_id: string, strategy_json: unknown) =>
  api<{ ok: boolean; address: string; strategy: unknown }>("/api/auth/run/start", {
    method: "POST",
    body: JSON.stringify({ wallet_id, strategy_json }),
  });

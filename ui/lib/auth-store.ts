import { atom } from "jotai";

export interface AuthUser {
  id: string;
  username: string;
}

export interface WalletInfo {
  id: string;
  label: string;
  address: string;
  created_at: number;
}

export interface StrategyInfo {
  id: string;
  name: string;
  wallet_id: string | null;
  wallet_label?: string;
  wallet_address?: string;
  updated_at: number;
  created_at: number;
}

export const authUserAtom = atom<AuthUser | null>(null);
export const authLoadingAtom = atom<boolean>(true);
export const isAuthenticatedAtom = atom((get) => get(authUserAtom) !== null);

export const walletsAtom = atom<WalletInfo[]>([]);
export const selectedWalletIdAtom = atom<string | null>(null);
export const selectedWalletAtom = atom((get) => {
  const wallets = get(walletsAtom);
  const id = get(selectedWalletIdAtom);
  return wallets.find((w) => w.id === id) ?? null;
});

export const strategiesAtom = atom<StrategyInfo[]>([]);

// User config (persisted key-value)
export const userConfigAtom = atom<Record<string, string>>({});

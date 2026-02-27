"use client";

import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import { tokenAtom, authUserAtom, authLoadingAtom, walletsAtom, userConfigAtom, setTokenGetter } from "@/lib/auth-store";
import { listWallets, getConfig } from "@/lib/auth-api";

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const token = useAtomValue(tokenAtom);
  const setUser = useSetAtom(authUserAtom);
  const setLoading = useSetAtom(authLoadingAtom);
  const setWallets = useSetAtom(walletsAtom);
  const setConfig = useSetAtom(userConfigAtom);

  // Wire up the token getter for auth-api
  useEffect(() => {
    setTokenGetter(() => token);
  }, [token]);

  // On mount: check localStorage for persisted token
  useEffect(() => {
    const stored = localStorage.getItem("defi-flow-token");
    const storedUser = localStorage.getItem("defi-flow-user");
    if (stored && storedUser) {
      try {
        const user = JSON.parse(storedUser);
        // Re-hydrate token into the getter before any API calls
        setTokenGetter(() => stored);
        setUser(user);
        // Load wallets and config
        Promise.all([listWallets(), getConfig()])
          .then(([wallets, config]) => {
            setWallets(wallets);
            setConfig(config);
          })
          .catch(() => {
            // Token expired — clear
            localStorage.removeItem("defi-flow-token");
            localStorage.removeItem("defi-flow-user");
            setUser(null);
          });
      } catch {
        localStorage.removeItem("defi-flow-token");
        localStorage.removeItem("defi-flow-user");
      }
    }
    setLoading(false);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return <>{children}</>;
}

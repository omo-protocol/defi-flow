"use client";

import { SessionProvider, useSession } from "next-auth/react";
import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { authUserAtom, authLoadingAtom, walletsAtom, userConfigAtom } from "@/lib/auth-store";
import { listWallets, getConfig } from "@/lib/auth-api";

function AuthSync({ children }: { children: React.ReactNode }) {
  const { data: session, status } = useSession();
  const setUser = useSetAtom(authUserAtom);
  const setLoading = useSetAtom(authLoadingAtom);
  const setWallets = useSetAtom(walletsAtom);
  const setConfig = useSetAtom(userConfigAtom);

  useEffect(() => {
    if (status === "loading") return;

    if (session?.user) {
      setUser({ id: session.user.id, username: session.user.name ?? "" });
      // Load wallets and config
      Promise.all([listWallets(), getConfig()])
        .then(([wallets, config]) => {
          setWallets(wallets);
          setConfig(config);
        })
        .catch(() => {});
    } else {
      setUser(null);
      setWallets([]);
      setConfig({});
    }

    setLoading(false);
  }, [session, status, setUser, setLoading, setWallets, setConfig]);

  return <>{children}</>;
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  return (
    <SessionProvider>
      <AuthSync>{children}</AuthSync>
    </SessionProvider>
  );
}

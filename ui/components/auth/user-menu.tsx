"use client";

import { useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  authUserAtom,
  authLoadingAtom,
  isAuthenticatedAtom,
  walletsAtom,
  strategiesAtom,
  tokenAtom,
} from "@/lib/auth-store";
import { LoginDialog } from "./login-dialog";
import { WalletManager } from "./wallet-manager";
import { StrategyPicker } from "./strategy-picker";
import { SettingsDialog } from "./settings-dialog";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { User, Wallet, FolderOpen, Settings, LogOut } from "lucide-react";
import { toast } from "sonner";

export function UserMenu() {
  const user = useAtomValue(authUserAtom);
  const loading = useAtomValue(authLoadingAtom);
  const isAuth = useAtomValue(isAuthenticatedAtom);
  const setUser = useSetAtom(authUserAtom);
  const setToken = useSetAtom(tokenAtom);
  const setWallets = useSetAtom(walletsAtom);
  const setStrategies = useSetAtom(strategiesAtom);

  const [loginOpen, setLoginOpen] = useState(false);
  const [walletsOpen, setWalletsOpen] = useState(false);
  const [strategiesOpen, setStrategiesOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const handleLogout = () => {
    setToken(null);
    setUser(null);
    setWallets([]);
    setStrategies([]);
    localStorage.removeItem("defi-flow-token");
    localStorage.removeItem("defi-flow-user");
    toast.success("Logged out");
  };

  if (loading) return null;

  if (!isAuth) {
    return (
      <>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={() => setLoginOpen(true)}
        >
          <User className="w-3.5 h-3.5 mr-1" />
          Sign In
        </Button>
        <LoginDialog open={loginOpen} onOpenChange={setLoginOpen} />
      </>
    );
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="h-7 px-2 text-xs">
            <User className="w-3.5 h-3.5 mr-1" />
            {user?.username}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem onClick={() => setWalletsOpen(true)} className="text-xs">
            <Wallet className="w-3.5 h-3.5 mr-2" />
            Wallets
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setStrategiesOpen(true)} className="text-xs">
            <FolderOpen className="w-3.5 h-3.5 mr-2" />
            Strategies
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setSettingsOpen(true)} className="text-xs">
            <Settings className="w-3.5 h-3.5 mr-2" />
            Settings
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={handleLogout} className="text-xs text-destructive">
            <LogOut className="w-3.5 h-3.5 mr-2" />
            Logout
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <WalletManager open={walletsOpen} onOpenChange={setWalletsOpen} />
      <StrategyPicker open={strategiesOpen} onOpenChange={setStrategiesOpen} />
      <SettingsDialog open={settingsOpen} onOpenChange={setSettingsOpen} />
    </>
  );
}

"use client";

import { useState } from "react";
import { useSetAtom } from "jotai";
import { register, login } from "@/lib/auth-api";
import { tokenAtom, authUserAtom, walletsAtom, userConfigAtom } from "@/lib/auth-store";
import { listWallets, getConfig } from "@/lib/auth-api";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { toast } from "sonner";

interface LoginDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function LoginDialog({ open, onOpenChange }: LoginDialogProps) {
  const [tab, setTab] = useState<string>("login");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const setToken = useSetAtom(tokenAtom);
  const setUser = useSetAtom(authUserAtom);
  const setWallets = useSetAtom(walletsAtom);
  const setConfig = useSetAtom(userConfigAtom);

  const reset = () => {
    setUsername("");
    setPassword("");
    setConfirmPassword("");
    setError("");
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);

    try {
      if (tab === "register") {
        if (password !== confirmPassword) {
          setError("Passwords do not match");
          setLoading(false);
          return;
        }
        await register(username, password);
      }

      // Login via Rust API
      const result = await login(username, password);
      setToken(result.token);
      setUser(result.user);

      // Persist to localStorage
      localStorage.setItem("defi-flow-token", result.token);
      localStorage.setItem("defi-flow-user", JSON.stringify(result.user));

      // Load wallets and config
      Promise.all([listWallets(), getConfig()])
        .then(([wallets, config]) => {
          setWallets(wallets);
          setConfig(config);
        })
        .catch(() => {});

      toast.success(tab === "register" ? `Welcome, ${username}!` : `Welcome back, ${username}!`);
      reset();
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Authentication failed");
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => { onOpenChange(v); if (!v) reset(); }}>
      <DialogContent className="sm:max-w-[380px]">
        <DialogHeader>
          <DialogTitle>Account</DialogTitle>
        </DialogHeader>
        <Tabs value={tab} onValueChange={(v) => { setTab(v); setError(""); }}>
          <TabsList className="grid w-full grid-cols-2">
            <TabsTrigger value="login">Login</TabsTrigger>
            <TabsTrigger value="register">Register</TabsTrigger>
          </TabsList>
          <form onSubmit={handleSubmit}>
            <TabsContent value="login" className="space-y-3 pt-2">
              <div className="space-y-1.5">
                <Label htmlFor="login-user" className="text-xs">Username</Label>
                <Input
                  id="login-user"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  autoComplete="username"
                  className="h-8 text-sm"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="login-pass" className="text-xs">Password</Label>
                <Input
                  id="login-pass"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="current-password"
                  className="h-8 text-sm"
                />
              </div>
            </TabsContent>
            <TabsContent value="register" className="space-y-3 pt-2">
              <div className="space-y-1.5">
                <Label htmlFor="reg-user" className="text-xs">Username</Label>
                <Input
                  id="reg-user"
                  value={username}
                  onChange={(e) => setUsername(e.target.value)}
                  autoComplete="username"
                  className="h-8 text-sm"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="reg-pass" className="text-xs">Password</Label>
                <Input
                  id="reg-pass"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  autoComplete="new-password"
                  className="h-8 text-sm"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="reg-confirm" className="text-xs">Confirm Password</Label>
                <Input
                  id="reg-confirm"
                  type="password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  autoComplete="new-password"
                  className="h-8 text-sm"
                />
              </div>
            </TabsContent>
            {error && (
              <p className="text-xs text-destructive mt-2">{error}</p>
            )}
            <Button type="submit" className="w-full mt-4 h-8 text-sm" disabled={loading}>
              {loading ? "..." : tab === "register" ? "Create Account" : "Sign In"}
            </Button>
          </form>
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

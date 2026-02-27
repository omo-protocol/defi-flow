"use client";

import { useState } from "react";
import { useAtom, useAtomValue } from "jotai";
import { walletsAtom, isAuthenticatedAtom } from "@/lib/auth-store";
import { createWallet, deleteWallet } from "@/lib/auth-api";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { MnemonicDisplay } from "./mnemonic-display";
import { Plus, Trash2, Copy } from "lucide-react";
import { toast } from "sonner";

interface WalletManagerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function WalletManager({ open, onOpenChange }: WalletManagerProps) {
  const [wallets, setWallets] = useAtom(walletsAtom);
  const isAuth = useAtomValue(isAuthenticatedAtom);
  const [showCreate, setShowCreate] = useState(false);
  const [mnemonic, setMnemonic] = useState<string | null>(null);
  const [newAddress, setNewAddress] = useState<string | null>(null);

  if (!isAuth) return null;

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-[420px] sm:max-w-[420px]">
        <SheetHeader>
          <SheetTitle className="text-sm">Wallets</SheetTitle>
        </SheetHeader>
        <div className="mt-4 space-y-3">
          {mnemonic && newAddress && (
            <MnemonicDisplay
              mnemonic={mnemonic}
              address={newAddress}
              onDismiss={() => { setMnemonic(null); setNewAddress(null); }}
            />
          )}

          {wallets.length === 0 && !showCreate && (
            <p className="text-xs text-muted-foreground text-center py-6">
              No wallets yet. Create or import one.
            </p>
          )}

          {wallets.map((w) => (
            <div
              key={w.id}
              className="flex items-center justify-between border rounded-md px-3 py-2"
            >
              <div className="min-w-0">
                <p className="text-xs font-medium truncate">{w.label}</p>
                <p className="text-[10px] text-muted-foreground font-mono truncate">
                  {w.address}
                </p>
              </div>
              <div className="flex items-center gap-1 ml-2">
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0"
                  onClick={() => {
                    navigator.clipboard.writeText(w.address);
                    toast.success("Address copied");
                  }}
                >
                  <Copy className="w-3 h-3" />
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0 text-destructive"
                  onClick={async () => {
                    try {
                      await deleteWallet(w.id);
                      setWallets(wallets.filter((x) => x.id !== w.id));
                      toast.success("Wallet deleted");
                    } catch (err) {
                      toast.error(err instanceof Error ? err.message : "Failed");
                    }
                  }}
                >
                  <Trash2 className="w-3 h-3" />
                </Button>
              </div>
            </div>
          ))}

          {showCreate ? (
            <CreateWalletForm
              onCreated={(wallet, mnemonic) => {
                setWallets([wallet, ...wallets]);
                setShowCreate(false);
                if (mnemonic) {
                  setMnemonic(mnemonic);
                  setNewAddress(wallet.address);
                }
                toast.success(`Wallet "${wallet.label}" created`);
              }}
              onCancel={() => setShowCreate(false)}
            />
          ) : (
            <Button
              variant="outline"
              size="sm"
              className="w-full h-8 text-xs"
              onClick={() => setShowCreate(true)}
            >
              <Plus className="w-3.5 h-3.5 mr-1" />
              Add Wallet
            </Button>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function CreateWalletForm({
  onCreated,
  onCancel,
}: {
  onCreated: (
    wallet: { id: string; label: string; address: string; created_at: number },
    mnemonic?: string
  ) => void;
  onCancel: () => void;
}) {
  const [mode, setMode] = useState<string>("generate");
  const [label, setLabel] = useState("");
  const [pk, setPk] = useState("");
  const [loading, setLoading] = useState(false);

  const handleCreate = async () => {
    if (!label.trim()) {
      toast.error("Label required");
      return;
    }
    setLoading(true);
    try {
      const result = await createWallet(
        label.trim(),
        mode as "generate" | "import",
        mode === "import" ? pk : undefined
      );
      onCreated(
        { id: result.id, label: result.label, address: result.address, created_at: Date.now() / 1000 },
        result.mnemonic
      );
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to create wallet");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="border rounded-md p-3 space-y-3">
      <Tabs value={mode} onValueChange={setMode}>
        <TabsList className="grid w-full grid-cols-2 h-7">
          <TabsTrigger value="generate" className="text-xs h-6">Generate</TabsTrigger>
          <TabsTrigger value="import" className="text-xs h-6">Import</TabsTrigger>
        </TabsList>
      </Tabs>
      <div className="space-y-1.5">
        <Label className="text-xs">Label</Label>
        <Input
          className="h-7 text-xs"
          placeholder="e.g. DN Strategy #1"
          value={label}
          onChange={(e) => setLabel(e.target.value)}
        />
      </div>
      {mode === "import" && (
        <div className="space-y-1.5">
          <Label className="text-xs">Private Key</Label>
          <Input
            className="h-7 text-xs font-mono"
            type="password"
            placeholder="0x..."
            value={pk}
            onChange={(e) => setPk(e.target.value)}
          />
        </div>
      )}
      <div className="flex gap-2">
        <Button
          size="sm"
          className="h-7 text-xs flex-1"
          onClick={handleCreate}
          disabled={loading}
        >
          {loading ? "..." : mode === "generate" ? "Generate" : "Import"}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          className="h-7 text-xs"
          onClick={onCancel}
        >
          Cancel
        </Button>
      </div>
    </div>
  );
}

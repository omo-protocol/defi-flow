"use client";

import { useState, useEffect } from "react";
import { useAtom } from "jotai";
import { userConfigAtom } from "@/lib/auth-store";
import { updateConfig } from "@/lib/auth-api";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Separator } from "@/components/ui/separator";
import { toast } from "sonner";

interface SettingsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

const CONFIG_SECTIONS = [
  {
    title: "API",
    fields: [
      { key: "api_url", label: "Engine API URL", placeholder: "http://localhost:8080", type: "text" },
      { key: "anthropic_api_key", label: "Anthropic API Key", placeholder: "sk-ant-...", type: "password" },
    ],
  },
  {
    title: "Backtest Defaults",
    fields: [
      { key: "default_capital", label: "Capital ($)", placeholder: "10000", type: "text" },
      { key: "default_slippage", label: "Slippage (bps)", placeholder: "10", type: "text" },
      { key: "default_seed", label: "Seed", placeholder: "42", type: "text" },
    ],
  },
  {
    title: "Execution Defaults",
    fields: [
      { key: "default_network", label: "Network", placeholder: "testnet", type: "text" },
    ],
  },
  {
    title: "RPC Overrides",
    fields: [
      { key: "rpc_hyperevm", label: "HyperEVM", placeholder: "https://rpc.hyperliquid.xyz/evm", type: "text" },
      { key: "rpc_ethereum", label: "Ethereum", placeholder: "https://eth.llamarpc.com", type: "text" },
      { key: "rpc_arbitrum", label: "Arbitrum", placeholder: "https://arb1.arbitrum.io/rpc", type: "text" },
      { key: "rpc_base", label: "Base", placeholder: "https://mainnet.base.org", type: "text" },
    ],
  },
] as const;

export function SettingsDialog({ open, onOpenChange }: SettingsDialogProps) {
  const [config, setConfig] = useAtom(userConfigAtom);
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) setDraft({ ...config });
  }, [open, config]);

  const handleSave = async () => {
    setSaving(true);
    try {
      // Compute diff — only send changed keys
      const updates: Record<string, string | null> = {};
      const allKeys = new Set([...Object.keys(config), ...Object.keys(draft)]);
      for (const key of allKeys) {
        const oldVal = config[key] || "";
        const newVal = draft[key] || "";
        if (oldVal !== newVal) {
          updates[key] = newVal || null; // null = delete
        }
      }

      if (Object.keys(updates).length === 0) {
        onOpenChange(false);
        return;
      }

      await updateConfig(updates);
      setConfig(draft);
      toast.success("Settings saved");
      onOpenChange(false);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[480px] max-h-[80vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="text-sm">Settings</DialogTitle>
        </DialogHeader>
        <div className="space-y-5 pt-2">
          {CONFIG_SECTIONS.map((section, i) => (
            <div key={section.title}>
              {i > 0 && <Separator className="mb-4" />}
              <p className="text-xs font-medium text-muted-foreground uppercase tracking-wider mb-3">
                {section.title}
              </p>
              <div className="space-y-3">
                {section.fields.map((field) => (
                  <div key={field.key}>
                    <Label className="text-xs">{field.label}</Label>
                    <Input
                      className="h-8 text-xs mt-1 font-mono"
                      type={field.type}
                      value={draft[field.key] || ""}
                      onChange={(e) =>
                        setDraft((d) => ({ ...d, [field.key]: e.target.value }))
                      }
                      placeholder={field.placeholder}
                    />
                  </div>
                ))}
              </div>
            </div>
          ))}

          <Separator />
          <Button
            onClick={handleSave}
            disabled={saving}
            size="sm"
            className="w-full"
          >
            {saving ? "Saving..." : "Save Settings"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

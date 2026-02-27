"use client";

import { useEffect, useState } from "react";
import { useAtom, useSetAtom } from "jotai";
import {
  strategiesAtom,
  walletsAtom,
  selectedWalletIdAtom,
} from "@/lib/auth-store";
import {
  nodesAtom,
  edgesAtom,
  workflowNameAtom,
  tokensManifestAtom,
  contractsManifestAtom,
} from "@/lib/workflow-store";
import {
  listStrategies,
  getStrategy,
  deleteStrategy,
  saveStrategy,
  updateStrategy,
} from "@/lib/auth-api";
import { convertCanvasToDefiFlow, convertDefiFlowToCanvas } from "@/lib/converters/canvas-defi-flow";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FolderOpen, Trash2, Save, Upload } from "lucide-react";
import { toast } from "sonner";
import { useReactFlow } from "@xyflow/react";

interface StrategyPickerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function StrategyPicker({ open, onOpenChange }: StrategyPickerProps) {
  const [strategies, setStrategies] = useAtom(strategiesAtom);
  const wallets = useAtom(walletsAtom)[0];
  const setSelectedWalletId = useSetAtom(selectedWalletIdAtom);
  const [nodes, setNodes] = useAtom(nodesAtom);
  const [edges, setEdges] = useAtom(edgesAtom);
  const [name, setName] = useAtom(workflowNameAtom);
  const [tokensManifest, setTokensManifest] = useAtom(tokensManifestAtom);
  const [contractsManifest, setContractsManifest] = useAtom(contractsManifestAtom);
  const { fitView } = useReactFlow();

  const [saving, setSaving] = useState(false);
  const [saveWalletId, setSaveWalletId] = useState<string>("");
  const [activeStrategyId, setActiveStrategyId] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      listStrategies().then(setStrategies).catch(() => {});
    }
  }, [open, setStrategies]);

  const handleLoad = async (id: string) => {
    try {
      const strat = await getStrategy(id);
      const workflow: DefiFlowWorkflow = JSON.parse(strat.workflow_json);
      const { nodes: newNodes, edges: newEdges, tokens, contracts } = convertDefiFlowToCanvas(workflow);
      setNodes(newNodes);
      setEdges(newEdges);
      setName(workflow.name || strat.name);
      setTokensManifest(tokens);
      setContractsManifest(contracts);
      setActiveStrategyId(id);
      if (strat.wallet_id) {
        setSelectedWalletId(strat.wallet_id);
      }
      onOpenChange(false);
      toast.success(`Loaded "${strat.name}"`);
      setTimeout(() => fitView({ padding: 0.2, duration: 300 }), 100);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to load");
    }
  };

  const handleSave = async () => {
    if (!saveWalletId) {
      toast.error("Wallet is required");
      return;
    }
    setSaving(true);
    try {
      const workflow = convertCanvasToDefiFlow(nodes, edges, name, undefined, tokensManifest, contractsManifest);
      if (activeStrategyId) {
        await updateStrategy(activeStrategyId, {
          name,
          workflow_json: workflow,
          wallet_id: saveWalletId,
        });
        toast.success("Strategy updated");
      } else {
        const result = await saveStrategy(name, workflow, saveWalletId);
        setActiveStrategyId(result.id);
        toast.success("Strategy saved");
      }
      setSelectedWalletId(saveWalletId);
      const updated = await listStrategies();
      setStrategies(updated);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteStrategy(id);
      setStrategies(strategies.filter((s) => s.id !== id));
      if (activeStrategyId === id) setActiveStrategyId(null);
      toast.success("Strategy deleted");
    } catch (err) {
      toast.error(err instanceof Error ? err.message : "Failed to delete");
    }
  };

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent className="w-[420px] sm:max-w-[420px]">
        <SheetHeader>
          <SheetTitle className="text-sm">Strategies</SheetTitle>
        </SheetHeader>
        <div className="mt-4 space-y-3">
          {/* Save current */}
          <div className="border rounded-md p-3 space-y-2">
            <p className="text-xs font-medium">
              {activeStrategyId ? "Update current strategy" : "Save current canvas"}
            </p>
            <div className="space-y-2">
              <div>
                <Label className="text-xs text-muted-foreground">
                  Wallet <span className="text-destructive">*</span>
                </Label>
                {wallets.length > 0 ? (
                  <Select value={saveWalletId} onValueChange={setSaveWalletId}>
                    <SelectTrigger className="h-7 text-xs mt-1">
                      <SelectValue placeholder="Select wallet" />
                    </SelectTrigger>
                    <SelectContent>
                      {wallets.map((w) => (
                        <SelectItem key={w.id} value={w.id} className="text-xs">
                          <span className="font-medium">{w.label}</span>
                          <span className="ml-2 text-muted-foreground font-mono">
                            {w.address.slice(0, 6)}...{w.address.slice(-4)}
                          </span>
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                ) : (
                  <p className="text-xs text-muted-foreground mt-1">
                    Add a wallet first (user menu).
                  </p>
                )}
              </div>
              <Button
                size="sm"
                className="h-7 text-xs w-full"
                onClick={handleSave}
                disabled={saving || !saveWalletId || wallets.length === 0}
              >
                <Save className="w-3 h-3 mr-1" />
                {saving ? "..." : activeStrategyId ? "Update" : "Save"}
              </Button>
            </div>
          </div>

          {/* List */}
          {strategies.length === 0 ? (
            <p className="text-xs text-muted-foreground text-center py-4">
              No saved strategies.
            </p>
          ) : (
            strategies.map((s) => (
              <div
                key={s.id}
                className={`flex items-center justify-between border rounded-md px-3 py-2 ${
                  s.id === activeStrategyId ? "border-primary/50 bg-primary/5" : ""
                }`}
              >
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-medium truncate">{s.name}</p>
                  <div className="flex items-center gap-2 mt-0.5">
                    {s.wallet_label && (
                      <span className="text-[10px] text-muted-foreground">
                        {s.wallet_label}
                      </span>
                    )}
                    <span className="text-[10px] text-muted-foreground">
                      {new Date(s.updated_at * 1000).toLocaleDateString()}
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-1 ml-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-[10px]"
                    onClick={() => handleLoad(s.id)}
                  >
                    <Upload className="w-3 h-3 mr-1" />
                    Load
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 w-6 p-0 text-destructive"
                    onClick={() => handleDelete(s.id)}
                  >
                    <Trash2 className="w-3 h-3" />
                  </Button>
                </div>
              </div>
            ))
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}

"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Copy, Eye, EyeOff, AlertTriangle } from "lucide-react";
import { toast } from "sonner";

interface MnemonicDisplayProps {
  mnemonic: string;
  address: string;
  onDismiss: () => void;
}

export function MnemonicDisplay({ mnemonic, address, onDismiss }: MnemonicDisplayProps) {
  const [revealed, setRevealed] = useState(false);
  const words = mnemonic.split(" ");

  return (
    <div className="border border-amber-500/30 bg-amber-500/5 rounded-md p-3 space-y-3">
      <div className="flex items-start gap-2">
        <AlertTriangle className="w-4 h-4 text-amber-500 shrink-0 mt-0.5" />
        <div>
          <p className="text-xs font-medium text-amber-500">
            Save your recovery phrase
          </p>
          <p className="text-[10px] text-muted-foreground mt-0.5">
            This will only be shown once. Write it down and store it securely.
          </p>
        </div>
      </div>

      <div className="text-[10px] font-mono text-muted-foreground truncate">
        {address}
      </div>

      <div className="relative">
        {!revealed && (
          <div className="absolute inset-0 flex items-center justify-center bg-card/80 backdrop-blur-sm rounded z-10">
            <Button
              variant="outline"
              size="sm"
              className="h-7 text-xs"
              onClick={() => setRevealed(true)}
            >
              <Eye className="w-3 h-3 mr-1" />
              Reveal
            </Button>
          </div>
        )}
        <div className="grid grid-cols-3 gap-1.5">
          {words.map((word, i) => (
            <div
              key={i}
              className="bg-muted/50 rounded px-2 py-1 text-[10px] font-mono"
            >
              <span className="text-muted-foreground mr-1">{i + 1}.</span>
              {word}
            </div>
          ))}
        </div>
      </div>

      <div className="flex gap-2">
        <Button
          variant="outline"
          size="sm"
          className="h-7 text-xs flex-1"
          onClick={() => {
            navigator.clipboard.writeText(mnemonic);
            toast.success("Mnemonic copied");
          }}
        >
          <Copy className="w-3 h-3 mr-1" />
          Copy
        </Button>
        {revealed && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 text-xs"
            onClick={() => setRevealed(false)}
          >
            <EyeOff className="w-3 h-3 mr-1" />
            Hide
          </Button>
        )}
        <Button
          size="sm"
          className="h-7 text-xs"
          onClick={onDismiss}
        >
          Done
        </Button>
      </div>
    </div>
  );
}

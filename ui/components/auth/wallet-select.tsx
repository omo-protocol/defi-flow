"use client";

import { useAtom, useAtomValue } from "jotai";
import { walletsAtom, selectedWalletIdAtom } from "@/lib/auth-store";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

export function WalletSelect() {
  const wallets = useAtomValue(walletsAtom);
  const [selectedId, setSelectedId] = useAtom(selectedWalletIdAtom);

  if (wallets.length === 0) {
    return (
      <p className="text-[10px] text-muted-foreground">
        No wallets — create one in your profile.
      </p>
    );
  }

  return (
    <Select value={selectedId ?? ""} onValueChange={setSelectedId}>
      <SelectTrigger className="h-7 text-xs">
        <SelectValue placeholder="Select wallet" />
      </SelectTrigger>
      <SelectContent>
        {wallets.map((w) => (
          <SelectItem key={w.id} value={w.id} className="text-xs">
            <span className="font-medium">{w.label}</span>
            <span className="ml-2 text-muted-foreground font-mono text-[10px]">
              {w.address.slice(0, 6)}...{w.address.slice(-4)}
            </span>
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}

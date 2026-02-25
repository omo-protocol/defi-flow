"use client";

import { KNOWN_CHAINS, type Chain, type CronInterval, type Trigger } from "@/lib/types/defi-flow";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Checkbox } from "@/components/ui/checkbox";
import { useState } from "react";

// ── Chain selector ───────────────────────────────────────────────────

export function ChainSelect({
  value,
  onChange,
  label = "Chain",
}: {
  value: Chain | undefined;
  onChange: (chain: Chain) => void;
  label?: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs">{label}</Label>
      <Select
        value={value?.name ?? ""}
        onValueChange={(name) => {
          const chain = KNOWN_CHAINS.find((c) => c.name === name);
          if (chain) onChange(chain);
        }}
      >
        <SelectTrigger className="h-8 text-xs">
          <SelectValue placeholder="Select chain" />
        </SelectTrigger>
        <SelectContent>
          {KNOWN_CHAINS.map((c) => (
            <SelectItem key={c.name} value={c.name}>
              {c.name} ({c.chain_id})
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

// ── Trigger config ───────────────────────────────────────────────────

export function TriggerConfig({
  value,
  onChange,
}: {
  value: Trigger | undefined;
  onChange: (t: Trigger | undefined) => void;
}) {
  const enabled = !!value;

  return (
    <div className="space-y-2 pt-2 border-t border-border/50">
      <div className="flex items-center gap-2">
        <Checkbox
          id="trigger-enabled"
          checked={enabled}
          onCheckedChange={(checked) => {
            if (checked) {
              onChange({ type: "cron", interval: "daily" });
            } else {
              onChange(undefined);
            }
          }}
        />
        <Label htmlFor="trigger-enabled" className="text-xs">
          Periodic trigger
        </Label>
      </div>
      {enabled && value?.type === "cron" && (
        <Select
          value={value.interval}
          onValueChange={(interval) =>
            onChange({ type: "cron", interval: interval as CronInterval })
          }
        >
          <SelectTrigger className="h-8 text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="hourly">Hourly</SelectItem>
            <SelectItem value="daily">Daily</SelectItem>
            <SelectItem value="weekly">Weekly</SelectItem>
            <SelectItem value="monthly">Monthly</SelectItem>
          </SelectContent>
        </Select>
      )}
    </div>
  );
}

// ── Generic field ────────────────────────────────────────────────────

export function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs">{label}</Label>
      {children}
    </div>
  );
}

export function TextField({
  label,
  value,
  onChange,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
}) {
  return (
    <Field label={label}>
      <Input
        className="h-8 text-xs"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
      />
    </Field>
  );
}

export function NumberField({
  label,
  value,
  onChange,
  placeholder,
  step,
  min,
  max,
}: {
  label: string;
  value: number | undefined;
  onChange: (v: number | undefined) => void;
  placeholder?: string;
  step?: number;
  min?: number;
  max?: number;
}) {
  return (
    <Field label={label}>
      <Input
        className="h-8 text-xs"
        type="number"
        value={value ?? ""}
        onChange={(e) => {
          const v = e.target.value;
          onChange(v === "" ? undefined : Number(v));
        }}
        placeholder={placeholder}
        step={step}
        min={min}
        max={max}
      />
    </Field>
  );
}

export function SelectField<T extends string>({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <Field label={label}>
      <Select value={value} onValueChange={(v) => onChange(v as T)}>
        <SelectTrigger className="h-8 text-xs">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((opt) => (
            <SelectItem key={opt.value} value={opt.value}>
              {opt.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </Field>
  );
}

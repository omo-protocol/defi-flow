/**
 * WASM bridge — loads the defi-flow Rust engine compiled to WebAssembly.
 * Exposes validate, parse, and schema functions.
 *
 * Uses a fully-dynamic import path so the build succeeds even when
 * pkg/ is absent (Vercel, CI). WASM is browser-only.
 */

type WasmExports = {
  default: (opts: { module_or_path: string }) => Promise<void>;
  validate_workflow_json: (json: string) => string;
  parse_workflow_json: (json: string) => string;
  get_schema: () => string;
};

let wasm: WasmExports | null = null;
let initPromise: Promise<boolean> | null = null;

async function ensureInit(): Promise<boolean> {
  if (wasm) return true;
  if (typeof window === "undefined") return false;

  if (!initPromise) {
    initPromise = (async () => {
      try {
        // Load from /public — works on both local dev and Vercel
        // Variable path prevents TypeScript/bundler from resolving statically
        const jsPath = "/defi_flow.js";
        const mod = (await import(/* webpackIgnore: true */ jsPath)) as WasmExports;
        await mod.default({ module_or_path: "/defi_flow_bg.wasm" });
        wasm = mod;
        return true;
      } catch {
        console.warn("[wasm] WASM module unavailable — validation disabled");
        return false;
      }
    })();
  }
  return initPromise;
}

export type ValidationResult = {
  valid: boolean;
  errors?: string[];
};

/** Validate a workflow JSON string using the Rust validator. */
export async function validateWorkflow(
  json: string
): Promise<ValidationResult> {
  const ok = await ensureInit();
  if (!ok || !wasm) return { valid: true };
  const raw = wasm.validate_workflow_json(json);
  return JSON.parse(raw) as ValidationResult;
}

/** Parse and re-serialize a workflow JSON (normalizes it). */
export async function parseWorkflow(json: string): Promise<unknown> {
  const ok = await ensureInit();
  if (!ok || !wasm) return JSON.parse(json);
  const raw = wasm.parse_workflow_json(json);
  return JSON.parse(raw);
}

/** Get the JSON Schema for workflows. */
export async function getWorkflowSchema(): Promise<unknown> {
  const ok = await ensureInit();
  if (!ok || !wasm) return {};
  return JSON.parse(wasm.get_schema());
}

/**
 * WASM bridge — loads the defi-flow Rust engine compiled to WebAssembly.
 * Exposes validate, parse, and schema functions.
 */

import init, {
  validate_workflow_json,
  parse_workflow_json,
  get_schema,
} from "../pkg/defi_flow";

let ready = false;
let initPromise: Promise<void> | null = null;

async function ensureInit() {
  if (ready) return;
  if (!initPromise) {
    initPromise = init({ module_or_path: "/defi_flow_bg.wasm" }).then(() => {
      ready = true;
    });
  }
  await initPromise;
}

export type ValidationResult = {
  valid: boolean;
  errors?: string[];
};

/** Validate a workflow JSON string using the Rust validator. */
export async function validateWorkflow(json: string): Promise<ValidationResult> {
  await ensureInit();
  const raw = validate_workflow_json(json);
  return JSON.parse(raw) as ValidationResult;
}

/** Parse and re-serialize a workflow JSON (normalizes it). */
export async function parseWorkflow(json: string): Promise<unknown> {
  await ensureInit();
  const raw = parse_workflow_json(json);
  return JSON.parse(raw);
}

/** Get the JSON Schema for workflows. */
export async function getWorkflowSchema(): Promise<unknown> {
  await ensureInit();
  return JSON.parse(get_schema());
}

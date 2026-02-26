import { atom } from "jotai";
import type { DefiFlowWorkflow } from "@/lib/types/defi-flow";

export type ToolActivity = {
  id: string;
  name: string;
  args: string;
  status: "running" | "done" | "error";
};

export type Message = {
  role: "user" | "assistant";
  content: string;
  /** Reasoning/thinking tokens (from <think> blocks) */
  thinking?: string;
  /** If the assistant generated a workflow, store it here */
  workflow?: DefiFlowWorkflow;
  /** Validation errors for this message's workflow (if any) */
  validationErrors?: string[];
  /** Tool calls made during this assistant turn */
  toolActivities?: ToolActivity[];
};

// In-memory only — never persisted to localStorage
export const openaiKeyAtom = atom<string>("");
export const openaiBaseUrlAtom = atom<string>("https://api.openai.com/v1");
export const openaiModelAtom = atom<string>("gpt-4o");

// Chat state
export const messagesAtom = atom<Message[]>([]);
export const generatingAtom = atom<boolean>(false);

import { NextResponse } from "next/server";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";

export async function GET(_req: Request, { params }: { params: Promise<{ id: string }> }) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const { id } = await params;
  const db = getDb();

  const strategy = db
    .prepare(
      `SELECT s.*, w.label as wallet_label, w.address as wallet_address
       FROM strategies s
       LEFT JOIN wallets w ON s.wallet_id = w.id
       WHERE s.id = ? AND s.user_id = ?`
    )
    .get(id, session.user.id) as Record<string, unknown> | undefined;

  if (!strategy) {
    return NextResponse.json({ error: "Strategy not found" }, { status: 404 });
  }

  return NextResponse.json(strategy);
}

export async function PUT(req: Request, { params }: { params: Promise<{ id: string }> }) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const userId = session.user.id;
  const { id } = await params;
  const db = getDb();

  const existing = db
    .prepare("SELECT id FROM strategies WHERE id = ? AND user_id = ?")
    .get(id, userId);
  if (!existing) {
    return NextResponse.json({ error: "Strategy not found" }, { status: 404 });
  }

  const body = await req.json();
  const { name, workflow_json, wallet_id } = body;

  if (wallet_id) {
    const wallet = db
      .prepare("SELECT id FROM wallets WHERE id = ? AND user_id = ?")
      .get(wallet_id, userId);
    if (!wallet) {
      return NextResponse.json({ error: "Wallet not found" }, { status: 404 });
    }
  }

  const jsonStr =
    workflow_json !== undefined
      ? typeof workflow_json === "string"
        ? workflow_json
        : JSON.stringify(workflow_json)
      : undefined;

  const updates: string[] = [];
  const values: unknown[] = [];

  if (name !== undefined) {
    updates.push("name = ?");
    values.push(name);
  }
  if (jsonStr !== undefined) {
    updates.push("workflow_json = ?");
    values.push(jsonStr);
  }
  if (wallet_id !== undefined) {
    updates.push("wallet_id = ?");
    values.push(wallet_id);
  }

  if (updates.length === 0) {
    return NextResponse.json({ error: "No fields to update" }, { status: 400 });
  }

  updates.push("updated_at = unixepoch()");
  values.push(id, userId);

  db.prepare(
    `UPDATE strategies SET ${updates.join(", ")} WHERE id = ? AND user_id = ?`
  ).run(...values);

  return NextResponse.json({ ok: true });
}

export async function DELETE(_req: Request, { params }: { params: Promise<{ id: string }> }) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const { id } = await params;
  const db = getDb();

  const existing = db
    .prepare("SELECT id FROM strategies WHERE id = ? AND user_id = ?")
    .get(id, session.user.id);
  if (!existing) {
    return NextResponse.json({ error: "Strategy not found" }, { status: 404 });
  }

  db.prepare("DELETE FROM strategies WHERE id = ?").run(id);

  return NextResponse.json({ ok: true });
}

import { NextResponse } from "next/server";
import crypto from "crypto";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";

export async function GET() {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const db = getDb();
  const strategies = db
    .prepare(
      `SELECT s.id, s.name, s.wallet_id, s.updated_at, s.created_at, w.label as wallet_label, w.address as wallet_address
       FROM strategies s
       LEFT JOIN wallets w ON s.wallet_id = w.id
       WHERE s.user_id = ?
       ORDER BY s.updated_at DESC`
    )
    .all(session.user.id);

  return NextResponse.json(strategies);
}

export async function POST(req: Request) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const userId = session.user.id;
  const body = await req.json();
  const { name, workflow_json, wallet_id } = body;

  if (!name || !workflow_json) {
    return NextResponse.json({ error: "Name and workflow JSON required" }, { status: 400 });
  }

  if (wallet_id) {
    const db = getDb();
    const wallet = db
      .prepare("SELECT id FROM wallets WHERE id = ? AND user_id = ?")
      .get(wallet_id, userId);
    if (!wallet) {
      return NextResponse.json({ error: "Wallet not found" }, { status: 404 });
    }
  }

  const db = getDb();
  const id = crypto.randomUUID();
  const jsonStr = typeof workflow_json === "string" ? workflow_json : JSON.stringify(workflow_json);

  db.prepare(
    "INSERT INTO strategies (id, user_id, wallet_id, name, workflow_json) VALUES (?, ?, ?, ?, ?)"
  ).run(id, userId, wallet_id ?? null, name, jsonStr);

  return NextResponse.json({ id, name, wallet_id: wallet_id ?? null }, { status: 201 });
}

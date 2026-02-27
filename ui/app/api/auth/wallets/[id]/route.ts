import { NextResponse } from "next/server";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";

export async function DELETE(_req: Request, { params }: { params: Promise<{ id: string }> }) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const { id } = await params;
  const db = getDb();

  const wallet = db
    .prepare("SELECT id FROM wallets WHERE id = ? AND user_id = ?")
    .get(id, session.user.id);

  if (!wallet) {
    return NextResponse.json({ error: "Wallet not found" }, { status: 404 });
  }

  db.prepare("DELETE FROM wallets WHERE id = ?").run(id);

  return NextResponse.json({ ok: true });
}

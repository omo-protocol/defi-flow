import { NextResponse } from "next/server";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";

export async function GET() {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const db = getDb();
  const rows = db
    .prepare("SELECT key, value FROM user_config WHERE user_id = ?")
    .all(session.user.id) as { key: string; value: string }[];

  const config: Record<string, string> = {};
  for (const row of rows) {
    config[row.key] = row.value;
  }

  return NextResponse.json(config);
}

export async function PUT(req: Request) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const body = await req.json();
  if (typeof body !== "object" || body === null) {
    return NextResponse.json({ error: "Expected object" }, { status: 400 });
  }

  const userId = session.user.id;
  const db = getDb();
  const upsert = db.prepare(
    "INSERT INTO user_config (user_id, key, value) VALUES (?, ?, ?) ON CONFLICT(user_id, key) DO UPDATE SET value = excluded.value"
  );
  const remove = db.prepare(
    "DELETE FROM user_config WHERE user_id = ? AND key = ?"
  );

  const tx = db.transaction(() => {
    for (const [key, value] of Object.entries(body)) {
      if (value === null || value === "") {
        remove.run(userId, key);
      } else {
        upsert.run(userId, key, String(value));
      }
    }
  });
  tx();

  return NextResponse.json({ ok: true });
}

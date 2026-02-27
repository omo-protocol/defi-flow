import { NextResponse } from "next/server";
import crypto from "crypto";
import bcrypt from "bcryptjs";
import { getDb } from "@/lib/server/db";
import { generateSalt } from "@/lib/server/crypto";

export async function POST(req: Request) {
  const body = await req.json();
  const { username, password } = body;

  if (!username || !password) {
    return NextResponse.json({ error: "Username and password required" }, { status: 400 });
  }

  if (username.length < 3 || username.length > 32) {
    return NextResponse.json({ error: "Username must be 3-32 characters" }, { status: 400 });
  }

  if (password.length < 8) {
    return NextResponse.json({ error: "Password must be at least 8 characters" }, { status: 400 });
  }

  const db = getDb();

  const existing = db.prepare("SELECT id FROM users WHERE username = ?").get(username);
  if (existing) {
    return NextResponse.json({ error: "Username already taken" }, { status: 409 });
  }

  const userId = crypto.randomUUID();
  const passwordHash = bcrypt.hashSync(password, 12);
  const keySalt = generateSalt();

  db.prepare(
    "INSERT INTO users (id, username, password_hash, key_salt) VALUES (?, ?, ?, ?)"
  ).run(userId, username, passwordHash, keySalt);

  return NextResponse.json({ ok: true, username });
}

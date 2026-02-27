import { NextResponse } from "next/server";
import crypto from "crypto";
import { Wallet, HDNodeWallet } from "ethers";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";
import { encryptPK } from "@/lib/server/crypto";

export async function GET() {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const db = getDb();
  const wallets = db
    .prepare("SELECT id, label, address, created_at FROM wallets WHERE user_id = ? ORDER BY created_at DESC")
    .all(session.user.id);

  return NextResponse.json(wallets);
}

export async function POST(req: Request) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const userId = session.user.id;
  const db = getDb();

  // Get derived key from users table (cached on login)
  const user = db.prepare("SELECT derived_key FROM users WHERE id = ?").get(userId) as
    | { derived_key: string | null }
    | undefined;

  if (!user?.derived_key) {
    return NextResponse.json({ error: "Session expired — please log in again" }, { status: 401 });
  }

  const derivedKey = Buffer.from(user.derived_key, "base64");

  const body = await req.json();
  const { mode, label, privateKey } = body;

  if (!label) {
    return NextResponse.json({ error: "Label required" }, { status: 400 });
  }

  let wallet: Wallet | HDNodeWallet;
  let mnemonic: string | null = null;

  if (mode === "import") {
    if (!privateKey) {
      return NextResponse.json({ error: "Private key required for import" }, { status: 400 });
    }
    try {
      wallet = new Wallet(privateKey);
    } catch {
      return NextResponse.json({ error: "Invalid private key" }, { status: 400 });
    }
  } else {
    const hdWallet = HDNodeWallet.createRandom();
    wallet = hdWallet;
    mnemonic = hdWallet.mnemonic?.phrase ?? null;
  }

  const existing = db
    .prepare("SELECT id FROM wallets WHERE user_id = ? AND address = ?")
    .get(userId, wallet.address);
  if (existing) {
    return NextResponse.json({ error: "Wallet address already registered" }, { status: 409 });
  }

  const walletId = crypto.randomUUID();
  const encryptedPk = encryptPK(wallet.privateKey, derivedKey);

  db.prepare(
    "INSERT INTO wallets (id, user_id, label, address, encrypted_pk) VALUES (?, ?, ?, ?, ?)"
  ).run(walletId, userId, label, wallet.address, encryptedPk);

  const result: Record<string, unknown> = {
    id: walletId,
    label,
    address: wallet.address,
  };

  if (mnemonic) {
    result.mnemonic = mnemonic;
  }

  return NextResponse.json(result, { status: 201 });
}

import { NextResponse } from "next/server";
import { auth } from "@/auth";
import { getDb } from "@/lib/server/db";
import { decryptPK } from "@/lib/server/crypto";

export async function POST(req: Request) {
  const session = await auth();
  if (!session?.user?.id) {
    return NextResponse.json({ error: "Unauthorized" }, { status: 401 });
  }

  const userId = session.user.id;
  const db = getDb();

  // Get derived key from users table
  const user = db.prepare("SELECT derived_key FROM users WHERE id = ?").get(userId) as
    | { derived_key: string | null }
    | undefined;

  if (!user?.derived_key) {
    return NextResponse.json({ error: "Session expired — please log in again" }, { status: 401 });
  }

  const derivedKey = Buffer.from(user.derived_key, "base64");

  const body = await req.json();
  const { wallet_id, strategy_json } = body;

  if (!wallet_id || !strategy_json) {
    return NextResponse.json(
      { error: "wallet_id and strategy_json required" },
      { status: 400 }
    );
  }

  const wallet = db
    .prepare("SELECT encrypted_pk, address FROM wallets WHERE id = ? AND user_id = ?")
    .get(wallet_id, userId) as { encrypted_pk: string; address: string } | undefined;

  if (!wallet) {
    return NextResponse.json({ error: "Wallet not found" }, { status: 404 });
  }

  let privateKey: string;
  try {
    privateKey = decryptPK(wallet.encrypted_pk, derivedKey);
  } catch {
    return NextResponse.json({ error: "Failed to decrypt wallet" }, { status: 500 });
  }

  // Inject the private key into the strategy JSON wallet node
  const strategy =
    typeof strategy_json === "string" ? JSON.parse(strategy_json) : strategy_json;

  const walletNode = strategy.nodes?.find(
    (n: Record<string, unknown>) => n.type === "wallet"
  );
  if (walletNode) {
    walletNode.address = wallet.address;
    walletNode.private_key = privateKey;
  }

  return NextResponse.json({
    ok: true,
    address: wallet.address,
    strategy,
  });
}

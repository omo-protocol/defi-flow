import { NextRequest, NextResponse } from "next/server";

const EXPLORER_APIS: Record<string, string> = {
  ethereum: "https://api.etherscan.io/api",
  base: "https://api.basescan.org/api",
  arbitrum: "https://api.arbiscan.io/api",
  optimism: "https://api-optimistic.etherscan.io/api",
  mantle: "https://api.mantlescan.xyz/api",
};

export async function GET(req: NextRequest) {
  const address = req.nextUrl.searchParams.get("address");
  const chain = req.nextUrl.searchParams.get("chain")?.toLowerCase();
  const apiKey = process.env.ETHERSCAN_API_KEY;

  if (!address || !chain) {
    return NextResponse.json(
      { error: "Missing address or chain" },
      { status: 400 },
    );
  }
  if (!apiKey) {
    return NextResponse.json(
      { error: "ETHERSCAN_API_KEY not configured" },
      { status: 500 },
    );
  }

  const baseUrl = EXPLORER_APIS[chain];
  if (!baseUrl) {
    return NextResponse.json(
      { error: `Unsupported chain "${chain}". Supported: ${Object.keys(EXPLORER_APIS).join(", ")}` },
      { status: 400 },
    );
  }

  const url = `${baseUrl}?module=contract&action=getsourcecode&address=${encodeURIComponent(address)}&apikey=${apiKey}`;
  const res = await fetch(url);
  if (!res.ok) {
    return NextResponse.json(
      { error: `Explorer API ${res.status}: ${res.statusText}` },
      { status: res.status },
    );
  }

  const data = await res.json();
  if (data.status !== "1" || !data.result?.[0]) {
    return NextResponse.json(
      { error: data.message || "Contract not found" },
      { status: 404 },
    );
  }

  const info = data.result[0];
  const isVerified = info.ABI !== "Contract source code not verified";

  // Extract function signatures from ABI (keep it compact)
  let methods: string[] = [];
  if (isVerified) {
    try {
      const abi = JSON.parse(info.ABI);
      methods = abi
        .filter((e: { type: string }) => e.type === "function")
        .map((e: { name: string; inputs: { type: string }[] }) => {
          const inputs = e.inputs?.map((i: { type: string }) => i.type).join(", ") ?? "";
          return `${e.name}(${inputs})`;
        });
    } catch {
      // ABI parse failed — skip
    }
  }

  return NextResponse.json({
    contractName: info.ContractName || null,
    isVerified,
    isProxy: info.Proxy === "1",
    implementation: info.Implementation || null,
    compiler: info.CompilerVersion || null,
    methods,
  });
}

import { NextRequest, NextResponse } from "next/server";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Pool = Record<string, any>;

export async function GET(req: NextRequest) {
  const action = req.nextUrl.searchParams.get("action");

  if (action === "yields") return handleYields(req);
  if (action === "protocol") return handleProtocol(req);

  return NextResponse.json(
    { error: 'Missing or invalid action. Use "yields" or "protocol".' },
    { status: 400 },
  );
}

async function handleYields(req: NextRequest) {
  const project = req.nextUrl.searchParams.get("project")?.toLowerCase();
  const chain = req.nextUrl.searchParams.get("chain")?.toLowerCase();
  const asset = req.nextUrl.searchParams.get("asset")?.toUpperCase();
  const stableOnly = req.nextUrl.searchParams.get("stablecoins_only") === "true";

  const res = await fetch("https://yields.llama.fi/pools");
  if (!res.ok) {
    return NextResponse.json(
      { error: `DeFiLlama API ${res.status}: ${res.statusText}` },
      { status: res.status },
    );
  }

  const { data } = (await res.json()) as { data: Pool[] };

  const filtered = data.filter((p: Pool) => {
    if (project && !p.project?.toLowerCase().includes(project)) return false;
    if (chain && p.chain?.toLowerCase() !== chain) return false;
    if (asset && !p.symbol?.toUpperCase().includes(asset)) return false;
    if (stableOnly && !p.stablecoin) return false;
    if ((p.tvlUsd ?? 0) < 10_000) return false; // skip dust pools
    return true;
  });

  filtered.sort((a: Pool, b: Pool) => (b.tvlUsd ?? 0) - (a.tvlUsd ?? 0));

  const results = filtered.slice(0, 10).map((p: Pool) => ({
    pool: p.pool,
    project: p.project,
    chain: p.chain,
    symbol: p.symbol,
    apy: p.apy,
    apyBase: p.apyBase,
    apyReward: p.apyReward,
    tvlUsd: p.tvlUsd,
    stablecoin: p.stablecoin,
    ilRisk: p.ilRisk,
  }));

  return NextResponse.json({ results });
}

async function handleProtocol(req: NextRequest) {
  const slug = req.nextUrl.searchParams.get("slug");
  if (!slug) {
    return NextResponse.json({ error: "Missing slug" }, { status: 400 });
  }

  const res = await fetch(`https://api.llama.fi/protocol/${encodeURIComponent(slug)}`);
  if (!res.ok) {
    return NextResponse.json(
      { error: `DeFiLlama API ${res.status}: ${res.statusText}` },
      { status: res.status },
    );
  }

  const data = await res.json();

  return NextResponse.json({
    name: data.name,
    slug: data.slug,
    chains: data.chains,
    tvl: data.tvl,
    category: data.category,
    url: data.url,
    description: data.description,
  });
}

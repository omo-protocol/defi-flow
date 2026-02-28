import { NextRequest, NextResponse } from "next/server";

export async function GET(req: NextRequest) {
  const query = req.nextUrl.searchParams.get("q");
  const apiKey = process.env.BRAVE_API_KEY;

  if (!query) {
    return NextResponse.json({ error: "Missing query" }, { status: 400 });
  }
  if (!apiKey) {
    return NextResponse.json({ error: "BRAVE_API_KEY not configured" }, { status: 500 });
  }

  const url = `https://api.search.brave.com/res/v1/web/search?q=${encodeURIComponent(query)}&count=5`;
  const res = await fetch(url, {
    headers: {
      Accept: "application/json",
      "Accept-Encoding": "gzip",
      "X-Subscription-Token": apiKey,
    },
  });

  if (!res.ok) {
    return NextResponse.json(
      { error: `Brave API ${res.status}: ${res.statusText}` },
      { status: res.status },
    );
  }

  const data = await res.json();
  return NextResponse.json(data);
}

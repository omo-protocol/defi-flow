import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  turbopack: {},
  serverExternalPackages: ["better-sqlite3"],
};

export default nextConfig;

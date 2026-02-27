import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  output: "standalone",
  turbopack: {},
  async rewrites() {
    const api = process.env.API_URL || "http://65.21.54.3:8080";
    return [
      {
        source: "/api/:path*",
        destination: `${api}/api/:path*`,
      },
      {
        source: "/health",
        destination: `${api}/health`,
      },
    ];
  },
};

export default nextConfig;

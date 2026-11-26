import type { NextConfig } from "next";

// Static export: `npm run build` writes web/out, which `hermit serve` hosts at /.
const config: NextConfig = {
  output: "export",
  trailingSlash: true,
  images: { unoptimized: true },
};

export default config;

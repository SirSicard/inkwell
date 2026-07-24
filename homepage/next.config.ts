import path from "node:path";
import { fileURLToPath } from "node:url";
import type { NextConfig } from "next";

// The homepage lives inside the app repo, which has its own package-lock.json
// one level up. Without an explicit root Turbopack infers the *parent* as the
// workspace root and warns on every build. Pin it to this directory.
const here = path.dirname(fileURLToPath(import.meta.url));

const nextConfig: NextConfig = {
  turbopack: {
    root: here,
  },
};

export default nextConfig;

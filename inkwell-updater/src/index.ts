/**
 * Inkwell Update Endpoint
 * 
 * Cloudflare Worker that serves Tauri updater responses.
 * Checks if a newer version exists in KV and returns the update manifest.
 * Returns 204 (no update) when current version is latest.
 * 
 * KV namespace: INKWELL_RELEASES
 * 
 * To publish a new version, add a KV entry:
 *   Key: "latest"
 *   Value: JSON manifest (see UpdateManifest type below)
 * 
 * Example KV value for "latest":
 * {
 *   "version": "0.2.0",
 *   "notes": "Bug fixes and performance improvements",
 *   "pub_date": "2026-04-01T12:00:00Z",
 *   "platforms": {
 *     "windows-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/inkwell_0.2.0_x64-setup.nsis.zip",
 *       "signature": "<base64 signature from .sig file>"
 *     },
 *     "darwin-aarch64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/inkwell_0.2.0_aarch64.app.tar.gz",
 *       "signature": "<base64 signature>"
 *     },
 *     "darwin-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/inkwell_0.2.0_x64.app.tar.gz",
 *       "signature": "<base64 signature>"
 *     },
 *     "linux-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/inkwell_0.2.0_amd64.AppImage.tar.gz",
 *       "signature": "<base64 signature>"
 *     }
 *   }
 * }
 */

export interface Env {
  INKWELL_RELEASES: KVNamespace;
}

interface PlatformEntry {
  url: string;
  signature: string;
}

interface UpdateManifest {
  version: string;
  notes: string;
  pub_date: string;
  platforms: Record<string, PlatformEntry>;
}

function compareVersions(a: string, b: string): number {
  const pa = a.replace(/^v/, "").split(".").map(Number);
  const pb = b.replace(/^v/, "").split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const na = pa[i] || 0;
    const nb = pb[i] || 0;
    if (na > nb) return 1;
    if (na < nb) return -1;
  }
  return 0;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const corsHeaders = {
      "Access-Control-Allow-Origin": "*",
      "Content-Type": "application/json",
    };

    if (request.method === "OPTIONS") {
      return new Response(null, {
        headers: {
          ...corsHeaders,
          "Access-Control-Allow-Methods": "GET, OPTIONS",
          "Access-Control-Max-Age": "86400",
        },
      });
    }

    // Parse URL: /api/update/{target}/{arch}/{current_version}
    const url = new URL(request.url);
    const match = url.pathname.match(/^\/api\/update\/([^/]+)\/([^/]+)\/([^/]+)$/);

    if (!match) {
      return new Response(
        JSON.stringify({ error: "Invalid path. Expected /api/update/{target}/{arch}/{current_version}" }),
        { status: 400, headers: corsHeaders }
      );
    }

    const [, target, arch, currentVersion] = match;
    const platformKey = `${target}-${arch}`;

    // Get latest release manifest from KV
    const raw = await env.INKWELL_RELEASES.get("latest");

    if (!raw) {
      // No release published yet
      return new Response(null, { status: 204 });
    }

    let manifest: UpdateManifest;
    try {
      manifest = JSON.parse(raw);
    } catch {
      return new Response(
        JSON.stringify({ error: "Invalid manifest in KV" }),
        { status: 500, headers: corsHeaders }
      );
    }

    // Check if update is needed
    if (compareVersions(manifest.version, currentVersion) <= 0) {
      // Current version is up to date
      return new Response(null, { status: 204 });
    }

    // Check if platform is supported
    const platform = manifest.platforms[platformKey];
    if (!platform) {
      // No build for this platform
      return new Response(null, { status: 204 });
    }

    // Return Tauri updater response
    return new Response(
      JSON.stringify({
        version: manifest.version,
        notes: manifest.notes,
        pub_date: manifest.pub_date,
        url: platform.url,
        signature: platform.signature,
      }),
      { status: 200, headers: corsHeaders }
    );
  },
};

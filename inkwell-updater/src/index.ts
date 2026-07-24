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
 * Artifact names follow productName ("Inkwell") and bundle.createUpdaterArtifacts
 * = true in tauri.conf.json, so Windows ships the setup .exe directly rather than
 * the v1-compatible .nsis.zip. Each "signature" is the literal contents of the
 * .sig file the release build publishes next to the artifact.
 *
 * Example KV value for "latest":
 * {
 *   "version": "0.2.0",
 *   "notes": "Bug fixes and performance improvements",
 *   "pub_date": "2026-04-01T12:00:00Z",
 *   "platforms": {
 *     "windows-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/Inkwell_0.2.0_x64-setup.exe",
 *       "signature": "<contents of the .sig file>"
 *     },
 *     "darwin-aarch64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/Inkwell_aarch64.app.tar.gz",
 *       "signature": "<contents of the .sig file>"
 *     },
 *     "darwin-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/Inkwell_x64.app.tar.gz",
 *       "signature": "<contents of the .sig file>"
 *     },
 *     "linux-x86_64": {
 *       "url": "https://github.com/SirSicard/inkwell/releases/download/v0.2.0/Inkwell_0.2.0_amd64.AppImage",
 *       "signature": "<contents of the .sig file>"
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

function parseVersion(version: string): { nums: number[]; pre: string } {
  const raw = version.replace(/^v/, "");
  const dash = raw.indexOf("-");
  const core = dash === -1 ? raw : raw.slice(0, dash);
  const pre = dash === -1 ? "" : raw.slice(dash + 1);
  const nums = core.split(".").map((part) => {
    const parsed = Number.parseInt(part, 10);
    return Number.isFinite(parsed) ? parsed : 0;
  });
  return { nums, pre };
}

// A pre-release sorts below the release it leads to. Number() on a raw "0-rc1"
// segment used to yield NaN, which made every comparison false and reported
// "up to date" forever to anyone running a pre-release build.
function compareVersions(a: string, b: string): number {
  const va = parseVersion(a);
  const vb = parseVersion(b);
  for (let i = 0; i < Math.max(va.nums.length, vb.nums.length); i++) {
    const na = va.nums[i] ?? 0;
    const nb = vb.nums[i] ?? 0;
    if (na > nb) return 1;
    if (na < nb) return -1;
  }
  if (va.pre && !vb.pre) return -1;
  if (!va.pre && vb.pre) return 1;
  if (va.pre !== vb.pre) return va.pre < vb.pre ? -1 : 1;
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

    // Get latest release manifest from KV. Every running client polls this, so
    // cache it at the edge; a five minute lag on a new release is invisible.
    const raw = await env.INKWELL_RELEASES.get("latest", { cacheTtl: 300 });

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

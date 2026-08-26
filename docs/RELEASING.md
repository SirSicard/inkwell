# Releasing

The chain used to be six manual steps held in one person's head, and two of
them failed silently in production: the updater manifest push exited without
writing (0.2.6, unnoticed for two days) and the canonical URL stayed pinned to
the previous build (0.2.5 and 0.2.6). Both now either automate themselves or
refuse to lie about having worked.

## First, prove it builds

**Nothing compiles this repository on push.** `build.yml` triggers on `v*` tags
and on manual dispatch only, so between one release and the next, Windows and
Linux are never built and `cargo test` never runs anywhere but a laptop. The
first cross-platform compile of a month's work would otherwise be the release
itself, with the tag already pushed.

A manual run is the dry run for this. Every publishing step in the workflow is
gated on `refs/tags/`, `tagName` resolves to an empty string off a tag, and
there is a dedicated step that uploads the bundles as artifacts instead. So this
builds all four platforms, notarises the macOS dmgs, and publishes nothing:

```bash
gh workflow run build.yml --ref main
gh run watch "$(gh run list --workflow build.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
```

Do this before touching a version number. A failure here costs a re-push; the
same failure after tagging costs a deleted tag and a burnt version.

## Cut it

```bash
# 1. Bump the version in all five places, plus Cargo.lock
#    src-tauri/Cargo.toml, src-tauri/tauri.conf.json, package.json,
#    homepage/lib/constants.ts, and the CHANGELOG heading
#    (cd src-tauri && cargo check)   regenerates Cargo.lock

# 2. Parity check. This is the one that only fails under the Tauri CLI:
#    a Rust/npm Tauri version mismatch compiles fine and dies in CI.
npm run tauri info | sed -n '/Packages/,/^$/p'

# 3. Tag and push. CI builds four platforms, notarises both dmgs, and
#    self-verifies with stapler validate plus spctl before uploading.
git tag -a vX.Y.Z -m "Inkwell X.Y.Z" && git push origin vX.Y.Z
```

## After CI goes green

```bash
# 4. Verify the artefact the way a user's Mac will: fresh download, browser
#    quarantine flag, then ask Gatekeeper. A green notarisation submission is
#    not a passing verdict; that exact gap shipped as 0.2.5.
#    (see the verify-dmg snippet in this file's history, or run by hand:)
xattr -w com.apple.quarantine "0081;$(printf %x $(date +%s));Safari;" Inkwell_X.Y.Z_aarch64.dmg
spctl --assess -vv --type open --context context:primary-signature Inkwell_X.Y.Z_aarch64.dmg

# 5. Publish
gh release edit vX.Y.Z --draft=false --latest

# 6. Push the updater manifest into Cloudflare KV. Retries once and then reads
#    the value back, so it cannot report success without having written.
inkwell-updater/publish-latest.sh

# 7. Point the cask at the release. Refuses on a draft, on a no-op rewrite,
#    and on a URL that does not return 200.
bin/update-cask.sh
```

## What no longer needs doing

**The homepage.** The `inkwell` Vercel project is connected to this repository
with root directory `homepage`, so pushing to `main` deploys it. "Include files
outside the root directory" is off and "Skip deployments" is on, which is what
stops a Rust-only commit from rebuilding the site: the homepage is
self-contained (own lockfile, no imports above its own directory), so nothing
outside it can affect the build.

**Moving the alias.** `getinkwell.vercel.app` is a project domain bound to the
Production environment, so it follows the newest production deployment on its
own. It used to be a hand-pinned alias, which is why `vercel --prod` moved the
three auto-generated domains and left the canonical URL a release behind.

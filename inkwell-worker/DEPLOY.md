# Deploy Inkwell Polish Worker

## Prerequisites
- Cloudflare account (free tier works)
- Groq API key
- Node.js 18+

## Steps

```bash
cd inkwell-worker
npm install

# 1. Login to Cloudflare
npx wrangler login

# 2. Create KV namespace
npx wrangler kv namespace create INKWELL_USAGE
# Copy the ID into wrangler.toml

# 3. Set the Groq API key as a secret
npx wrangler secret put GROQ_API_KEY
# Paste your Groq key when prompted

# 4. Deploy
npm run deploy
```

## After Deploy
- Worker URL will be: `https://inkwell-polish.<your-subdomain>.workers.dev`
- Update `PROXY_BASE_URL` in `src-tauri/src/llm.rs` to match
- Test: `curl -X POST https://inkwell-polish.<subdomain>.workers.dev/v1/polish -H "Content-Type: application/json" -d '{"install_id":"test","text":"um hello uh my name is mattias"}'`

## Limits
- 4,000 words/week per install_id
- 5,000 character max per request
- KV entries auto-expire after 8 days
- Cloudflare free tier: 100K requests/day

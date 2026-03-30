/**
 * Inkwell AI Polish Proxy
 * 
 * Cloudflare Worker that proxies transcription text to Groq for cleanup.
 * Rate-limited per install_id: 4,000 words/week via KV.
 * 
 * KV namespace: INKWELL_USAGE
 * Secret: GROQ_API_KEY
 */

export interface Env {
  INKWELL_USAGE: KVNamespace;
  GROQ_API_KEY: string;
}

interface PolishRequest {
  install_id: string;
  text: string;
  system_prompt?: string;
}

interface UsageRecord {
  week_start: string;
  words_used: number;
}

const WEEKLY_LIMIT = 4000;
const MODEL = "llama-3.3-70b-versatile";
const MAX_INPUT_LENGTH = 5000; // characters

function currentMonday(): string {
  const now = new Date();
  const day = now.getUTCDay(); // 0=Sun
  const diff = day === 0 ? 6 : day - 1; // days since Monday
  const monday = new Date(now);
  monday.setUTCDate(now.getUTCDate() - diff);
  return monday.toISOString().slice(0, 10);
}

function countWords(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

async function getUsage(kv: KVNamespace, installId: string): Promise<UsageRecord> {
  const key = `usage:${installId}`;
  const raw = await kv.get(key);
  const monday = currentMonday();

  if (raw) {
    try {
      const record: UsageRecord = JSON.parse(raw);
      if (record.week_start === monday) {
        return record;
      }
    } catch {}
  }

  return { week_start: monday, words_used: 0 };
}

async function saveUsage(kv: KVNamespace, installId: string, record: UsageRecord): Promise<void> {
  const key = `usage:${installId}`;
  // TTL: 8 days (auto-cleanup old weeks)
  await kv.put(key, JSON.stringify(record), { expirationTtl: 691200 });
}

const DEFAULT_PROMPT =
  "Clean up this speech-to-text transcription. Fix grammar, punctuation, and capitalization. " +
  "Remove filler words (um, uh, like) and false starts. Keep the speaker's original meaning, " +
  "tone, and word choices. Do NOT add, remove, or rephrase content. Return ONLY the cleaned text.";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    // CORS preflight
    if (request.method === "OPTIONS") {
      return new Response(null, {
        headers: {
          "Access-Control-Allow-Origin": "*",
          "Access-Control-Allow-Methods": "POST, OPTIONS",
          "Access-Control-Allow-Headers": "Content-Type, X-Inkwell-Version",
          "Access-Control-Max-Age": "86400",
        },
      });
    }

    const corsHeaders = {
      "Access-Control-Allow-Origin": "*",
      "Content-Type": "application/json",
    };

    // Only POST /v1/polish
    const url = new URL(request.url);
    if (request.method !== "POST" || url.pathname !== "/v1/polish") {
      return new Response(
        JSON.stringify({ error: "Not found" }),
        { status: 404, headers: corsHeaders }
      );
    }

    // Parse body
    let body: PolishRequest;
    try {
      body = await request.json();
    } catch {
      return new Response(
        JSON.stringify({ error: "Invalid JSON" }),
        { status: 400, headers: corsHeaders }
      );
    }

    if (!body.install_id || !body.text) {
      return new Response(
        JSON.stringify({ error: "Missing install_id or text" }),
        { status: 400, headers: corsHeaders }
      );
    }

    // Input length guard
    if (body.text.length > MAX_INPUT_LENGTH) {
      return new Response(
        JSON.stringify({ error: "Text too long", max_chars: MAX_INPUT_LENGTH }),
        { status: 413, headers: corsHeaders }
      );
    }

    // Check usage
    const usage = await getUsage(env.INKWELL_USAGE, body.install_id);
    const inputWords = countWords(body.text);

    if (usage.words_used + inputWords > WEEKLY_LIMIT) {
      const monday = new Date(usage.week_start);
      monday.setUTCDate(monday.getUTCDate() + 7);
      return new Response(
        JSON.stringify({
          error: "rate_limit",
          message: `Weekly limit reached (${WEEKLY_LIMIT} words). Resets ${monday.toISOString().slice(0, 10)}.`,
          words_used: usage.words_used,
          limit: WEEKLY_LIMIT,
          remaining: Math.max(0, WEEKLY_LIMIT - usage.words_used),
          resets_on: monday.toISOString().slice(0, 10),
        }),
        { status: 429, headers: corsHeaders }
      );
    }

    // Call Groq
    const systemPrompt = body.system_prompt || DEFAULT_PROMPT;

    let groqResp: Response;
    try {
      groqResp = await fetch("https://api.groq.com/openai/v1/chat/completions", {
        method: "POST",
        headers: {
          "Authorization": `Bearer ${env.GROQ_API_KEY}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          model: MODEL,
          messages: [
            { role: "system", content: systemPrompt },
            { role: "user", content: body.text },
          ],
          max_tokens: 1024,
          temperature: 0.3,
        }),
      });
    } catch (e: any) {
      return new Response(
        JSON.stringify({ error: "Upstream request failed", message: e.message }),
        { status: 502, headers: corsHeaders }
      );
    }

    if (!groqResp.ok) {
      const errBody = await groqResp.text();
      return new Response(
        JSON.stringify({ error: "Upstream error", status: groqResp.status, message: errBody }),
        { status: 502, headers: corsHeaders }
      );
    }

    const groqData: any = await groqResp.json();
    const polished = groqData.choices?.[0]?.message?.content?.trim() || "";
    const outputWords = countWords(polished);

    // Update usage (count output words, same as client)
    usage.words_used += outputWords;
    await saveUsage(env.INKWELL_USAGE, body.install_id, usage);

    return new Response(
      JSON.stringify({
        text: polished,
        words_used: usage.words_used,
        limit: WEEKLY_LIMIT,
        remaining: Math.max(0, WEEKLY_LIMIT - usage.words_used),
        resets_on: (() => {
          const monday = new Date(usage.week_start);
          monday.setUTCDate(monday.getUTCDate() + 7);
          return monday.toISOString().slice(0, 10);
        })(),
      }),
      { status: 200, headers: corsHeaders }
    );
  },
};

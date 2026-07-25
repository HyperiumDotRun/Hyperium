// Hyperium Ondo market-data proxy.
// Deploy as a Cloudflare Worker. Set a secret named ONDO_API_KEY in the
// Cloudflare dashboard (Worker -> Settings -> Variables -> add secret) —
// the read-only key never needs to live in the client, in this repo, or
// in any chat/tool history.
//
// Route: GET /market/:symbol  (symbol already normalized, e.g. TSLAon)
// Caches each symbol at the edge for 30s so many concurrent Hyperium users
// asking about the same ticker collapse into one upstream call.

const ONDO_BASE = "https://api.gm.ondo.finance/v1";
const CACHE_SECONDS = 30;
const SYMBOL_RE = /^[A-Za-z0-9]{1,15}$/;

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    const match = url.pathname.match(/^\/market\/([^/]+)$/);

    if (request.method !== "GET" || !match) {
      return json({ error: "not found" }, 404);
    }

    const symbol = match[1];
    if (!SYMBOL_RE.test(symbol)) {
      return json({ error: "invalid symbol" }, 400);
    }

    const cache = caches.default;
    const cacheKey = new Request(url.toString(), request);
    const cached = await cache.match(cacheKey);
    if (cached) return cached;

    const upstream = await fetch(`${ONDO_BASE}/assets/${symbol}/market`, {
      headers: { "x-api-key": env.ONDO_API_KEY },
    });

    const body = await upstream.text();
    const resp = new Response(body, {
      status: upstream.status,
      headers: {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
        "cache-control": `public, max-age=${CACHE_SECONDS}`,
      },
    });

    if (upstream.ok) ctx.waitUntil(cache.put(cacheKey, resp.clone()));
    return resp;
  },
};

function json(obj, status) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" },
  });
}

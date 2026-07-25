// Hyperium Ondo market-data proxy.
// Deploy as a Cloudflare Worker. Set a secret named ONDO_API_KEY in the
// Cloudflare dashboard (Worker -> Settings -> Variables -> add secret) —
// the read-only key never needs to live in the client, in this repo, or
// in any chat/tool history.
//
// Routes:
//   GET /market/:symbol     one symbol's market data (already normalized,
//                           e.g. TSLAon), cached at the edge per-symbol for
//                           30s.
//   GET /market-all         every supported asset in one call
//                           (upstream: GET /v1/assets/all/market), cached at
//                           the edge for 20s — this is the bulk pull
//                           ondo_screen uses to cover the whole watchlist
//                           instead of one request per symbol.
//   GET /dividends/:symbol   one symbol's dividend info (yield, payout
//                            frequency, last cash amount/date), cached for
//                            1 hour — this data changes at most quarterly,
//                            nowhere near as often as a price.
//   GET /multiplier/:symbol  one symbol's shares-multiplier history
//                            (upstream: GET /v1/assets/{symbol}/shares-multiplier),
//                            forwarding the required ?range= query param
//                            (1day/1month/3month/6month/1year/all). This is
//                            the real historical trail of how dividends have
//                            compounded into the token's price - cached for
//                            1 hour, same reasoning as dividends.
// All routes collapse concurrent Hyperium users into one upstream call per
// cache window, so this stays cheap against Ondo's own rate limits.

const ONDO_BASE = "https://api.gm.ondo.finance/v1";
const CACHE_SECONDS = 30;
const ALL_CACHE_SECONDS = 20;
const DIVIDEND_CACHE_SECONDS = 3600;
const MULTIPLIER_CACHE_SECONDS = 3600;
const SYMBOL_RE = /^[A-Za-z0-9]{1,15}$/;
const RANGE_RE = /^(1day|1month|3month|6month|1year|all)$/;

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);

    if (request.method !== "GET") {
      return json({ error: "not found" }, 404);
    }

    if (url.pathname === "/market-all") {
      return proxy(url, request, ctx, env, "/assets/all/market", ALL_CACHE_SECONDS);
    }

    const marketMatch = url.pathname.match(/^\/market\/([^/]+)$/);
    if (marketMatch) {
      const symbol = marketMatch[1];
      if (!SYMBOL_RE.test(symbol)) {
        return json({ error: "invalid symbol" }, 400);
      }
      return proxy(url, request, ctx, env, `/assets/${symbol}/market`, CACHE_SECONDS);
    }

    const dividendMatch = url.pathname.match(/^\/dividends\/([^/]+)$/);
    if (dividendMatch) {
      const symbol = dividendMatch[1];
      if (!SYMBOL_RE.test(symbol)) {
        return json({ error: "invalid symbol" }, 400);
      }
      return proxy(url, request, ctx, env, `/assets/${symbol}/dividends`, DIVIDEND_CACHE_SECONDS);
    }

    const multiplierMatch = url.pathname.match(/^\/multiplier\/([^/]+)$/);
    if (multiplierMatch) {
      const symbol = multiplierMatch[1];
      const range = url.searchParams.get("range") || "1year";
      if (!SYMBOL_RE.test(symbol)) {
        return json({ error: "invalid symbol" }, 400);
      }
      if (!RANGE_RE.test(range)) {
        return json({ error: "invalid range" }, 400);
      }
      return proxy(
        url,
        request,
        ctx,
        env,
        `/assets/${symbol}/shares-multiplier?range=${range}`,
        MULTIPLIER_CACHE_SECONDS,
      );
    }

    return json({ error: "not found" }, 404);
  },
};

async function proxy(url, request, ctx, env, upstreamPath, cacheSeconds) {
  const cache = caches.default;
  const cacheKey = new Request(url.toString(), request);
  const cached = await cache.match(cacheKey);
  if (cached) return cached;

  const upstream = await fetch(`${ONDO_BASE}${upstreamPath}`, {
    headers: { "x-api-key": env.ONDO_API_KEY },
  });

  const body = await upstream.text();
  const resp = new Response(body, {
    status: upstream.status,
    headers: {
      "content-type": "application/json",
      "access-control-allow-origin": "*",
      "cache-control": `public, max-age=${cacheSeconds}`,
    },
  });

  if (upstream.ok) ctx.waitUntil(cache.put(cacheKey, resp.clone()));
  return resp;
}

function json(obj, status) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { "content-type": "application/json" },
  });
}

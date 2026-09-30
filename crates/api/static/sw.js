/* ApexIntel — Service Worker v1.1.0
 *
 * Routing rules (B349 / audit):
 *   - /api/*     → never intercepted (network only by the browser): the
 *                  previous static-extension check ran first, so
 *                  /api/openapi.json was served cache-first.
 *   - /static/*  → cache-first (non-user assets only).
 *   - navigations → network only with an offline fallback page. PDF/CSV
 *                  downloads are navigations and must never be cached; they
 *                  would survive logout on a shared machine.
 *   - everything else is not intercepted and never cached.
 */

const STATIC_CACHE = 'apexintel-static-v2';

/* ─── Assets to pre-cache on install ─────────────────────────────────── */
const PRECACHE_URLS = [
  '/static/css/tailwind.css',
  '/static/js/htmx.min.js',
  '/static/js/app.js',
  '/static/js/sse-client.js',
  '/static/manifest.json'
];

/* ─── Install: pre-cache static assets ───────────────────────────────── */
self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(STATIC_CACHE).then((cache) => cache.addAll(PRECACHE_URLS))
  );
  self.skipWaiting();
});

/* ─── Activate: drop every cache except the current static assets ────── */
self.addEventListener('activate', (event) => {
  const validCaches = new Set([STATIC_CACHE]);
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames.filter((name) => !validCaches.has(name)).map((name) => caches.delete(name))
      );
    }).then(() => self.clients.claim())
  );
});

/* ─── Fetch routing ──────────────────────────────────────────────────── */
self.addEventListener('fetch', (event) => {
  const { request } = event;
  const url = new URL(request.url);

  // Only same-origin GETs; never intercept the service worker itself.
  if (url.origin !== self.location.origin) return;
  if (request.method !== 'GET') return;
  if (url.pathname === '/sw.js') return;

  // API responses are dynamic and authenticated: never cached.
  if (url.pathname.startsWith('/api/')) {
    event.respondWith(networkOnly(request));
    return;
  }

  // Static, non-user assets only.
  if (url.pathname.startsWith('/static/')) {
    event.respondWith(cacheFirst(request));
    return;
  }

  // Navigations (including /insights/:id/pdf and export downloads) are
  // network-only with an offline page; nothing else is intercepted or cached.
  if (request.mode === 'navigate') {
    event.respondWith(fetch(request).catch(() => offlinePage()));
  }
});

/* ─── Cache-first strategy (static assets) ───────────────────────────── */
async function cacheFirst(request) {
  const cached = await caches.match(request);
  if (cached) return cached;

  try {
    const response = await fetch(request);
    if (response.ok && response.type === 'basic') {
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, response.clone());
    }
    return response;
  } catch (err) {
    return new Response('Offline', { status: 503 });
  }
}

/* ─── Network-only strategy ──────────────────────────────────────────── */
async function networkOnly(request) {
  try {
    return await fetch(request);
  } catch (err) {
    return new Response(JSON.stringify({ error: 'offline' }), {
      status: 503,
      headers: { 'Content-Type': 'application/json' }
    });
  }
}

/* ─── Offline fallback page ──────────────────────────────────────────── */
function offlinePage() {
  return new Response(
    '<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">' +
    '<title>Offline — ApexIntel</title>' +
    '<style>body{font-family:system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#0f172a;color:#e2e8f0;text-align:center;padding:2rem}' +
    'h1{font-size:1.5rem;margin-bottom:0.5rem}p{color:#94a3b8;max-width:24rem}</style>' +
    '</head><body>' +
    '<h1>You\'re offline</h1>' +
    '<p>ApexIntel needs a network connection to load this page. Please check your connection and try again.</p>' +
    '</body></html>',
    { status: 503, headers: { 'Content-Type': 'text/html; charset=utf-8' } }
  );
}

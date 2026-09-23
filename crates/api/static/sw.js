/* ApexIntel — Service Worker v1.0.0 */

const CACHE_NAME = 'apexintel-v1';
const STATIC_CACHE = 'apexintel-static-v1';

/* ─── Assets to pre-cache on install ─────────────────────────────────── */
/* Never pre-cache '/' or any HTML: navigations are authenticated and must
   not persist in Cache Storage (B349). Only static, non-user assets here. */
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
    caches.open(STATIC_CACHE).then((cache) => {
      return cache.addAll(PRECACHE_URLS);
    })
  );
  // Activate immediately — don't wait for page refresh
  self.skipWaiting();
});

/* ─── Activate: clean up old caches ──────────────────────────────────── */
self.addEventListener('activate', (event) => {
  const validCaches = new Set([CACHE_NAME, STATIC_CACHE]);
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames
          .filter((name) => !validCaches.has(name))
          .map((name) => caches.delete(name))
      );
    }).then(() => self.clients.claim())
  );
});

/* ─── Fetch: network-first for HTML, cache-first for static ──────────── */
self.addEventListener('fetch', (event) => {
  const { request } = event;
  const url = new URL(request.url);

  // Only handle same-origin requests
  if (url.origin !== self.location.origin) return;

  // Skip non-GET and service worker itself
  if (request.method !== 'GET') return;
  if (url.pathname === '/sw.js') return;

  // ── Static assets (CSS, JS, images, fonts, manifest): cache-first ──
  if (
    url.pathname.startsWith('/static/') ||
    url.pathname.match(/\.(css|js|svg|woff2?|ttf|png|jpg|ico|json)$/)
  ) {
    event.respondWith(cacheFirst(request));
    return;
  }

  // ── API requests: network-only (never cache dynamic data) ────────────
  if (url.pathname.startsWith('/api/')) {
    event.respondWith(networkOnly(request));
    return;
  }

  // ── HTML / navigation requests: network-first, fallback to cache ────
  if (request.mode === 'navigate' || request.destination === 'document') {
    event.respondWith(networkFirst(request));
    return;
  }

  // ── Everything else: network-first ──────────────────────────────────
  event.respondWith(networkFirst(request));
});

/* ─── Cache-first strategy ───────────────────────────────────────────── */
async function cacheFirst(request) {
  const cached = await caches.match(request);
  if (cached) return cached;

  try {
    const response = await fetch(request);
    if (response.ok) {
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, response.clone());
    }
    return response;
  } catch (err) {
    return new Response('Offline', { status: 503 });
  }
}

/* ─── Network-first strategy ─────────────────────────────────────────── */
async function networkFirst(request) {
  try {
    const response = await fetch(request);
    // B349: never cache HTML documents — pages are authenticated and the
    // cache survives logout on shared machines.
    const isHtml = (response.headers.get('content-type') || '').includes('text/html');
    if (response.ok && response.type === 'basic' && !isHtml) {
      const cache = await caches.open(CACHE_NAME);
      cache.put(request, response.clone());
    }
    return response;
  } catch (err) {
    const cached = await caches.match(request);
    if (cached) return cached;
    // Offline fallback for navigation
    if (request.mode === 'navigate') {
      return new Response(
        '<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">' +
        '<title>Offline — ApexIntel</title>' +
        '<style>body{font-family:system-ui,sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;margin:0;background:#0f172a;color:#e2e8f0;text-align:center;padding:2rem}' +
        'h1{font-size:1.5rem;margin-bottom:0.5rem}p{color:#94a3b8;max-width:24rem}</style>' +
        '</head><body>' +
        '<h1>🔌 You\'re offline</h1>' +
        '<p>ApexIntel needs a network connection to load this page. Please check your connection and try again.</p>' +
        '</body></html>',
        { status: 503, headers: { 'Content-Type': 'text/html; charset=utf-8' } }
      );
    }
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

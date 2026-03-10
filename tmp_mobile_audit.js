const fs = require('fs');
const crypto = require('crypto');
const { chromium } = require('playwright');

const baseURL = process.env.PLAYWRIGHT_BASE_URL;
const username = process.env.PLAYWRIGHT_USER;
const sessionSecret = process.env.PLAYWRIGHT_SESSION_SECRET || '94eaff21c290ca4ff353ed53470995cbbb2f2899b073bde2ada4d7d85ed97143';

if (!baseURL || !username || !sessionSecret) {
  console.error(JSON.stringify({ error: 'missing env', baseURL: !!baseURL, username: !!username, sessionSecret: !!sessionSecret }));
  process.exit(1);
}

const pages = ['/', '/warnings', '/insights', '/companies', '/persons', '/graph', '/security', '/admin', '/memos', '/recipes'];
const viewports = [
  { name: 'iphone-se', width: 320, height: 568 },
  { name: 'iphone-12', width: 390, height: 844 },
];

function buildSessionToken() {
  const payloadBytes = Buffer.from(JSON.stringify({ sub: username, iat: Date.now() }), 'utf8');
  const payloadB64 = payloadBytes.toString('base64url');
  const signature = crypto.createHmac('sha256', sessionSecret).update(payloadBytes).digest('hex');
  return `${payloadB64}.${signature}`;
}

(async () => {
  const browser = await chromium.launch({ headless: true });
  const results = [];
  const token = buildSessionToken();
  const cookieDomain = new URL(baseURL).hostname;

  for (const viewport of viewports) {
    const context = await browser.newContext({ viewport, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
    await context.addCookies([
      {
        name: 'apex_session',
        value: token,
        domain: cookieDomain,
        path: '/',
        httpOnly: true,
        secure: true,
        sameSite: 'Lax',
      },
    ]);
    const page = await context.newPage();

    for (const path of pages) {
      await page.goto(baseURL + path, { waitUntil: 'networkidle', timeout: 30000 });
      await page.waitForTimeout(500);
      const audit = await page.evaluate(() => {
        const body = document.body;
        const docEl = document.documentElement;
        const viewportWidth = window.innerWidth;
        const pageOverflow = Math.max(body.scrollWidth, docEl.scrollWidth) - viewportWidth;
        const elements = [...document.querySelectorAll('body *')];

        const suspicious = elements
          .map((el) => {
            const rect = el.getBoundingClientRect();
            const text = (el.textContent || '').replace(/\s+/g, ' ').trim();
            const style = window.getComputedStyle(el);
            if (!text || text.length < 8) return null;
            if (rect.width <= 0 || rect.height <= 0) return null;
            if (style.display === 'none' || style.visibility === 'hidden') return null;
            const looksTall = rect.height > rect.width * 2.2;
            const narrow = rect.width < 42;
            const manyChars = text.length >= 12;
            if (!(looksTall && narrow && manyChars)) return null;
            return {
              tag: el.tagName.toLowerCase(),
              width: Math.round(rect.width),
              height: Math.round(rect.height),
              text: text.slice(0, 80),
              classes: (el.className || '').toString().slice(0, 160),
            };
          })
          .filter(Boolean)
          .slice(0, 12);

        const hardOverflowNodes = elements
          .map((el) => {
            const rect = el.getBoundingClientRect();
            const style = window.getComputedStyle(el);
            if (rect.width <= viewportWidth + 12) return null;
            const wrapped = !!el.closest('.overflow-x-auto, .overflow-auto, [class*="overflow-x-auto"], [class*="overflow-auto"]');
            if (wrapped) return null;
            if (style.position === 'fixed') return null;
            return {
              tag: el.tagName.toLowerCase(),
              width: Math.round(rect.width),
              text: ((el.textContent || '').replace(/\s+/g, ' ').trim()).slice(0, 80),
              classes: (el.className || '').toString().slice(0, 160),
            };
          })
          .filter(Boolean)
          .slice(0, 12);

        return { title: document.title, pageOverflow, suspicious, hardOverflowNodes };
      });

      results.push({ viewport: viewport.name, path, ...audit });
    }

    await context.close();
  }

  await browser.close();
  const outputPath = '/Users/sabelakhoua/IdeaProjects/ApexIntel/tmp_mobile_audit_results.json';
  fs.writeFileSync(outputPath, JSON.stringify(results, null, 2));
  console.log(JSON.stringify({ wrote: outputPath, records: results.length }));
})().catch((error) => {
  console.error(error);
  process.exit(1);
});

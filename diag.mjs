import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const url = 'https://apexmail.ee/solutions/enterprise/';
await p.goto(url, { waitUntil: 'networkidle' });
await p.waitForTimeout(800);
const diag = await p.evaluate(() => {
  const out = { main: {}, content: {}, tables: [], issues: [] };
  const main = document.querySelector('main');
  const cs = (el) => getComputedStyle(el);
  const h1 = document.querySelector('main h1');
  out.main.display = cs(main).display;
  out.main.maxWidth = cs(main).maxWidth;
  out.main.padding = cs(main).padding;
  out.main.fontFamily = cs(main).fontFamily;
  out.h1 = { size: cs(h1).fontSize, weight: cs(h1).fontWeight, color: cs(h1).color, margin: cs(h1).margin, family: cs(h1).fontFamily };
  // main content children classes
  out.mainChildren = Array.from(main.children).slice(0, 12).map(c => ({ tag: c.tagName, cls: c.className.slice(0, 80) }));
  document.querySelectorAll('main table').forEach((t, i) => {
    out.tables.push({ i, w: t.offsetWidth, cssW: cs(t).width, borderCollapse: cs(t).borderCollapse, bg: cs(t).backgroundColor });
  });
  // any element with obvious unapplied styles?
  const h2 = document.querySelector('main h2');
  if (h2) out.h2 = { size: cs(h2).fontSize, weight: cs(h2).fontWeight, color: cs(h2).color, margin: cs(h2).margin };
  const p1 = document.querySelector('main p');
  if (p1) out.p = { size: cs(p1).fontSize, lineHeight: cs(p1).lineHeight, color: cs(p1).color };
  return out;
});
console.log(JSON.stringify(diag, null, 1));
await browser.close();

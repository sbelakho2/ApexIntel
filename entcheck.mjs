import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
await p.goto('https://apexmail.ee/solutions/enterprise/', { waitUntil: 'networkidle', timeout: 30000 });
await p.waitForTimeout(1000);
const r = await p.evaluate(() => {
  const main = document.querySelector('main');
  const container = main.querySelector('.max-w-3xl');
  const prose = main.querySelector('.prose');
  const t1 = document.querySelector('main table');
  return {
    containerWidth: container ? container.offsetWidth : null,
    containerPadding: container ? getComputedStyle(container).padding : null,
    proseFound: !!prose,
    tableBorder: t1 ? getComputedStyle(t1.firstElementChild ? t1.firstElementChild.children[0] : t1).borderTopWidth : null,
    h1Count: main.querySelectorAll('h1').length,
    h2Count: main.querySelectorAll('h2').length,
    bodyScrollW: document.body.scrollWidth,
    viewport: window.innerWidth,
  };
});
console.log(JSON.stringify(r, null, 1));
await browser.close();

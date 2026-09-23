import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
await p.goto('https://apexmail.ee/solutions/enterprise/', { waitUntil: 'networkidle', timeout: 30000 });
await p.waitForTimeout(1000);
const r = await p.evaluate(() => {
  const t1 = document.querySelector('main table');
  const th = t1 ? t1.querySelector('th') : null;
  const td = t1 ? t1.querySelector('td') : null;
  return {
    tableWidth: t1 ? t1.offsetWidth : null,
    tableBorder: t1 ? getComputedStyle(t1).borderTopWidth : null,
    thBg: th ? getComputedStyle(th).backgroundColor : null,
    thBorder: th ? getComputedStyle(th).borderTopWidth : null,
    thFontSize: th ? getComputedStyle(th).fontSize : null,
    tdPadding: td ? getComputedStyle(td).padding : null,
  };
});
console.log(JSON.stringify(r, null, 1));
await p.screenshot({ path: '/tmp/enterprise-styled.png' });
await browser.close();

import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
for (const url of ['https://app.apexmail.ee/login/', 'https://admin.apexmail.ee/']) {
  await p.goto(url, { waitUntil: 'networkidle', timeout: 30000 });
  await p.waitForTimeout(800);
  const r = await p.evaluate(() => {
    const sheets = Array.from(document.styleSheets).map(s => { try { return (s.href || 'inline').split('/').pop(); } catch(e){ return 'ERR'; } });
    const h1 = document.querySelector('h1, h2');
    const btn = document.querySelector('button, a.btn, a[class*=btn]');
    return {
      url: location.pathname,
      sheets,
      heading: h1 ? h1.textContent.trim().slice(0, 40) : null,
      headingSize: h1 ? getComputedStyle(h1).fontSize : null,
      bodyFont: getComputedStyle(document.body).fontFamily,
      buttonText: btn ? btn.textContent.trim().slice(0, 25) : null,
      buttonBg: btn ? getComputedStyle(btn).backgroundColor : null,
    };
  });
  console.log(JSON.stringify(r));
}
await browser.close();

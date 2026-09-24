import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => {
  if (m.type() === 'error' && m.text().includes('Content Security')) {
    // Print the FULL violation message (usually includes the offending node context)
    console.log('VIOLATION:', m.text());
  }
});
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load', timeout: 30000 });
await p.waitForTimeout(5000);
// Find all elements with inline style attributes (the widget's own DOM should have none now)
const inlineStyles = await p.evaluate(() => {
  const results = [];
  document.querySelectorAll('[style]').forEach(el => {
    results.push({ tag: el.tagName, cls: (el.className||'').toString().slice(0, 40), style: el.getAttribute('style').slice(0, 60) });
  });
  return results;
});
console.log('elements with inline style:', JSON.stringify(inlineStyles.slice(0, 10), null, 1));
await browser.close();

import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext({ viewport: { width: 1440, height: 900 } })).newPage();
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'networkidle', timeout: 30000 });
await p.waitForTimeout(1000);
// find the sidebar and any collapse controls
const info = await p.evaluate(() => {
  const aside = document.querySelector('aside[aria-label*="sidebar" i], aside[data-sidebar-storage-key], .apex-console-sidebar');
  const hasShortcutContract = !!document.querySelector('[data-shortcut-action="toggle-sidebar"]');
  const scripts = Array.from(document.querySelectorAll('script[src]')).map(s => s.src);
  return {
    asideFound: !!aside,
    asideClass: aside ? aside.className.slice(0, 120) : null,
    hasToggleContract: hasShortcutContract,
    scripts,
    bodyTextSample: document.body.innerText.slice(0, 200),
  };
});
console.log(JSON.stringify(info, null, 1));
await browser.close();

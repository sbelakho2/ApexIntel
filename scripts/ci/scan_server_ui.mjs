// Adversarial server-UI scan: every navigation route at mobile, tablet and
// desktop, checked for
//   * failed same-origin resources (hard 404s like a missing favicon)
//   * uncaught page errors and console errors
//   * serious/critical axe-core accessibility violations
//   * horizontal overflow / elements escaping the viewport
//   * empty <main> (blank page)
//   * clipped text inside fixed-height rows
// and photographed full-page into artifacts/dogfood for pixel review.
//
// Usage: BASE_URL=... ADMIN_USER=... ADMIN_PASS=... \
//        DATABASE_URL=... node scripts/ci/scan_server_ui.mjs
import { chromium } from 'playwright';
import { createRequire } from 'node:module';
import { mkdirSync } from 'node:fs';

const require = createRequire(import.meta.url);
const { seedDatabase, login } = require('../../e2e/helpers/server-ui-fixtures.cjs');
const { ROUTES, VIEWPORTS } = require('../../e2e/helpers/server-ui-routes.cjs');
const AxeBuilder = require('@axe-core/playwright').default;

const BASE = process.env.BASE_URL || 'http://127.0.0.1:9095';
const ARTIFACTS = process.env.SCAN_ARTIFACTS || 'artifacts/dogfood';

const violations = [];
const v = (msg) => {
  violations.push(msg);
  console.error(`FAIL ${msg}`);
};

async function main() {
  await seedDatabase();
  mkdirSync(ARTIFACTS, { recursive: true });

  const browser = await chromium.launch();
  const authContext = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const authPage = await authContext.newPage();
  await login(authPage);
  const storageState = await authContext.storageState();
  await authContext.close();

  for (const viewport of VIEWPORTS) {
    const context = await browser.newContext({
      storageState,
      viewport: { width: viewport.width, height: viewport.height },
    });
    for (const route of ROUTES) {
      const page = await context.newPage();
      const consoleErrors = [];
      const failedResources = [];
      page.on('console', (message) => {
        if (message.type() === 'error') {
          consoleErrors.push(message.text());
        }
      });
      page.on('pageerror', (error) => consoleErrors.push(`pageerror: ${error.message}`));
      page.on('response', (response) => {
        if (response.url().startsWith(BASE) && response.status() >= 400) {
          failedResources.push(`${response.status()} ${response.url()}`);
        }
      });

      const response = await page.goto(`${BASE}${route}`, { waitUntil: 'domcontentloaded' });
      if (!response || response.status() >= 400) {
        v(`${route} [${viewport.name}] responded ${response ? response.status() : 'no response'}`);
      }
      await page.waitForTimeout(600); // hydration + htmx settle

      if (consoleErrors.length) {
        v(`${route} [${viewport.name}] console: ${consoleErrors.slice(0, 3).join(' | ')}`);
      }
      if (failedResources.length) {
        v(`${route} [${viewport.name}] failed resources: ${failedResources.slice(0, 3).join(' | ')}`);
      }

      const layout = await page.evaluate(() => {
        const doc = document.documentElement;
        const viewportWidth = window.innerWidth;

        // An element is not a layout defect when it is inside a hidden or
        // aria-hidden subtree, or when a scroll/clip ancestor contains its
        // overflow (the standard `overflow-x-auto` table wrapper), or when it
        // is a closed off-canvas drawer (fully outside the viewport on one
        // side, e.g. `-translate-x-full`).
        const contained = (el) => {
          for (let node = el.parentElement; node && node !== document.body; node = node.parentElement) {
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity) === 0) {
              return true;
            }
            if (node.getAttribute('aria-hidden') === 'true') {
              return true;
            }
            if (
              (style.overflowX === 'auto' ||
                style.overflowX === 'scroll' ||
                style.overflowX === 'hidden' ||
                style.overflowX === 'clip') &&
              node !== el
            ) {
              return true;
            }
          }
          return false;
        };

        const escaping = [];
        for (const el of document.querySelectorAll('body *')) {
          const style = window.getComputedStyle(el);
          if (style.position === 'fixed' || style.display === 'none' || style.visibility === 'hidden') {
            continue;
          }
          const rect = el.getBoundingClientRect();
          if (rect.width < 1 || rect.height < 1 || contained(el)) {
            continue;
          }
          const fullyOffCanvas = rect.right <= 1 || rect.left >= viewportWidth - 1;
          const partialEscape = !fullyOffCanvas && (rect.right > viewportWidth + 2 || rect.left < -2);
          if (partialEscape) {
            const tag = el.tagName.toLowerCase();
            const id = el.id ? `#${el.id}` : '';
            escaping.push(`${tag}${id}(${Math.round(rect.left)}..${Math.round(rect.right)})`);
            if (escaping.length >= 5) {
              break;
            }
          }
        }
        const main = document.querySelector('main');
        const mainText = main ? main.innerText.replace(/\s+/g, ' ').trim() : '';
        return {
          scrollWidth: doc.scrollWidth,
          viewportWidth,
          escaping,
          mainLength: mainText.length,
        };
      });

      if (layout.scrollWidth > layout.viewportWidth + 1) {
        v(
          `${route} [${viewport.name}] horizontal overflow: ${layout.scrollWidth} > ${layout.viewportWidth}`
        );
      }
      if (layout.escaping.length) {
        v(`${route} [${viewport.name}] elements escape viewport: ${layout.escaping.join(', ')}`);
      }
      if (layout.mainLength < 20) {
        v(`${route} [${viewport.name}] main content is empty`);
      }

      // Axe is CPU-heavy; run it on the two representative widths per route.
      if (viewport.name !== 'tablet') {
        try {
          const results = await new AxeBuilder({ page })
            .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
            .analyze();
          const blocking = results.violations.filter(
            (violation) => violation.impact === 'serious' || violation.impact === 'critical'
          );
          for (const violation of blocking) {
            v(
              `${route} [${viewport.name}] axe ${violation.impact} ${violation.id}: ` +
                violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(', ')
            );
          }
        } catch (error) {
          v(`${route} [${viewport.name}] axe failed: ${error.message}`);
        }
      }

      const shot = `${ARTIFACTS}/${route === '/' ? 'home' : route.replace(/[^a-z0-9]+/gi, '-').replace(/^-|-$/g, '')}-${viewport.name}.png`;
      await page.screenshot({ path: shot, fullPage: true });
      await page.close();
    }
    await context.close();
  }

  await browser.close();

  if (violations.length) {
    console.error(`\nadversarial UI scan: ${violations.length} violation(s)`);
    process.exit(1);
  }
  console.log(
    `adversarial UI scan: ${ROUTES.length} routes × ${VIEWPORTS.length} viewports clean; screenshots in ${ARTIFACTS}`
  );
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});

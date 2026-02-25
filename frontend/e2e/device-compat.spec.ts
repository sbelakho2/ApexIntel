import { test, expect, devices } from '@playwright/test';
import type { BrowserContextOptions } from 'playwright';

type DeviceProfile = {
  name: string;
  use: BrowserContextOptions;
};

function contextOptionsForDevice(name: string): BrowserContextOptions {
  const descriptor = devices[name];
  const { defaultBrowserType: _ignored, ...options } = descriptor;
  return options;
}

const routes = [
  '/',
  '/warnings',
  '/insights',
  '/memos',
  '/companies',
  '/persons',
  '/competitors',
  '/security',
  '/graph',
  '/recipes',
  '/settings',
] as const;

const profiles: DeviceProfile[] = [
  { name: 'iPhone SE', use: contextOptionsForDevice('iPhone SE') },
  { name: 'iPhone 12', use: contextOptionsForDevice('iPhone 12') },
  { name: 'Pixel 7', use: contextOptionsForDevice('Pixel 7') },
  { name: 'Galaxy S9+', use: contextOptionsForDevice('Galaxy S9+') },
  { name: 'iPad (gen 7)', use: contextOptionsForDevice('iPad (gen 7)') },
  { name: 'iPad Pro 11', use: contextOptionsForDevice('iPad Pro 11') },
  { name: 'Desktop 1536', use: { viewport: { width: 1536, height: 960 } } },
];

test.describe('Device compatibility audit', () => {
  test.setTimeout(8 * 60 * 1000);

  test('all core routes are compatible across devices', async ({ browser }) => {
    const findings: string[] = [];

    for (const profile of profiles) {
      const context = await browser.newContext(profile.use);
      const page = await context.newPage();

      for (const route of routes) {
        const runtimeErrors: string[] = [];

        page.removeAllListeners('console');
        page.removeAllListeners('pageerror');

        page.on('console', (message) => {
          if (message.type() !== 'error') return;
          const text = message.text();
          if (/favicon|hydration|Extra attributes from the server|Support for defaultProps will be removed/i.test(text)) {
            return;
          }
          runtimeErrors.push(`console:${text}`);
        });

        page.on('pageerror', (error) => {
          runtimeErrors.push(`pageerror:${error.message}`);
        });

        await page.goto(route, { waitUntil: 'domcontentloaded' });
        await page.waitForSelector('.apex-card', { timeout: 15000 });
        await page.waitForTimeout(100);

        const metrics = await page.evaluate(() => {
          const doc = document.documentElement;
          const body = document.body;
          const overflowX = Math.max(0, doc.scrollWidth - doc.clientWidth);
          const isCoarsePointer = window.matchMedia('(pointer: coarse)').matches || navigator.maxTouchPoints > 0;
          const minShortSide = isCoarsePointer ? 36 : 20;

          const tinyTargets = Array.from(document.querySelectorAll('button, input, select, textarea, [role="button"]'))
            .filter((el) => {
              if (!(el instanceof HTMLElement)) return false;
              if (el.classList.contains('sr-only')) return false;
              if (el.closest('svg')) return false;
              const rect = el.getBoundingClientRect();
              const style = getComputedStyle(el);
              if (rect.width <= 0 || rect.height <= 0) return false;
              if (style.display === 'none' || style.visibility === 'hidden') return false;
              return Math.min(rect.width, rect.height) < minShortSide;
            })
            .slice(0, 8)
            .map((el) => {
              const rect = el.getBoundingClientRect();
              return `${el.tagName.toLowerCase()}(${Math.round(rect.width)}x${Math.round(rect.height)})`;
            });

          const clipped = Array.from(body.querySelectorAll('*'))
            .filter((el) => {
              if (el.classList.contains('sr-only')) return false;
              if (el.tagName.toLowerCase() === 'g') return false;
              const style = getComputedStyle(el);
              if (style.textOverflow === 'ellipsis' || style.whiteSpace === 'nowrap') return false;
              const overflowHidden = /hidden|clip/.test(style.overflow) || /hidden|clip/.test(style.overflowX);
              if (!overflowHidden) return false;
              return el.scrollWidth - el.clientWidth > 12;
            })
            .slice(0, 6)
            .map((el) => `${el.tagName.toLowerCase()}.${String(el.className).slice(0, 40)}`);

          return { overflowX, tinyTargets, clipped };
        });

        if (runtimeErrors.length) {
          findings.push(`${profile.name} ${route} runtime-errors: ${runtimeErrors.slice(0, 2).join(' | ')}`);
        }
        if (metrics.overflowX > 2) {
          findings.push(`${profile.name} ${route} overflow-x:${metrics.overflowX}`);
        }
        if (metrics.tinyTargets.length) {
          findings.push(`${profile.name} ${route} tiny-targets:${metrics.tinyTargets.join(',')}`);
        }
        if (metrics.clipped.length) {
          findings.push(`${profile.name} ${route} clipped:${metrics.clipped.join(',')}`);
        }
      }

      await context.close();
    }

    expect(findings, `Device compatibility findings:\n${findings.join('\n')}`).toEqual([]);
  });
});

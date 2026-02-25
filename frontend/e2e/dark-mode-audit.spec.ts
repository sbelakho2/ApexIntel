import { test, expect, devices } from '@playwright/test';
import type { BrowserContextOptions } from 'playwright';

type DeviceProfile = {
  name: string;
  use: BrowserContextOptions;
};

function contextOptionsForDevice(name: string): BrowserContextOptions {
  const descriptor = devices[name];
  const { defaultBrowserType: _ignored, ...options } = descriptor;
  return { ...options, colorScheme: 'dark' };
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
  { name: 'Desktop 1536 dark', use: { viewport: { width: 1536, height: 960 }, colorScheme: 'dark' } },
  { name: 'iPhone 12 dark', use: contextOptionsForDevice('iPhone 12') },
  { name: 'iPad Pro 11 dark', use: contextOptionsForDevice('iPad Pro 11') },
];

test.describe('Dark mode contrast and visibility audit', () => {
  test.setTimeout(8 * 60 * 1000);

  test('core routes meet dark mode readability and coordination checks', async ({ browser }) => {
    const findings: string[] = [];

    for (const profile of profiles) {
      const context = await browser.newContext(profile.use);
      const page = await context.newPage();
      await page.addInitScript(() => {
        try {
          localStorage.setItem('theme', 'dark');
          document.documentElement.classList.add('dark');
        } catch {
          // no-op
        }
      });

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
        await page.evaluate(() => {
          document.documentElement.classList.add('dark');
        });
        await page.waitForSelector('.apex-card', { timeout: 15000 });
        await page.waitForTimeout(100);

        const metrics = await page.evaluate(() => {
          type RGBA = { r: number; g: number; b: number; a: number };

          const parseColor = (color: string): RGBA | null => {
            const normalized = color.trim().toLowerCase();
            const match = normalized.match(/rgba?\(([^)]+)\)/);
            if (!match) return null;
            const parts = match[1].split(',').map((part) => part.trim());
            if (parts.length < 3) return null;
            const r = Number(parts[0]);
            const g = Number(parts[1]);
            const b = Number(parts[2]);
            const a = parts.length >= 4 ? Number(parts[3]) : 1;
            if ([r, g, b, a].some((value) => Number.isNaN(value))) return null;
            return { r, g, b, a };
          };

          const toLinear = (value: number): number => {
            const n = value / 255;
            return n <= 0.03928 ? n / 12.92 : ((n + 0.055) / 1.055) ** 2.4;
          };

          const luminance = (rgb: RGBA): number => (
            0.2126 * toLinear(rgb.r) + 0.7152 * toLinear(rgb.g) + 0.0722 * toLinear(rgb.b)
          );

          const contrastRatio = (fg: RGBA, bg: RGBA): number => {
            const l1 = luminance(fg);
            const l2 = luminance(bg);
            const lighter = Math.max(l1, l2);
            const darker = Math.min(l1, l2);
            return (lighter + 0.05) / (darker + 0.05);
          };

          const resolveBackground = (el: Element): RGBA => {
            let current: Element | null = el;
            while (current) {
              const style = getComputedStyle(current);
              const parsed = parseColor(style.backgroundColor);
              if (parsed && parsed.a > 0.85) {
                return { ...parsed, a: 1 };
              }
              current = current.parentElement;
            }
            const bodyBg = parseColor(getComputedStyle(document.body).backgroundColor);
            return bodyBg ? { ...bodyBg, a: 1 } : { r: 0, g: 0, b: 0, a: 1 };
          };

          const visibleTextEls = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6,p,span,a,button,label,td,th,li,input,select,textarea'))
            .filter((el) => {
              const style = getComputedStyle(el);
              if (style.display === 'none' || style.visibility === 'hidden') return false;
              if (Number(style.opacity) < 0.4) return false;
              const rect = el.getBoundingClientRect();
              if (rect.width <= 0 || rect.height <= 0) return false;
              const text = (el.textContent ?? '').trim();
              if (el.tagName === 'INPUT' || el.tagName === 'SELECT' || el.tagName === 'TEXTAREA') return true;
              return text.length >= 2;
            });

          const lowContrast = visibleTextEls
            .map((el) => {
              const style = getComputedStyle(el);
              const fg = parseColor(style.color);
              if (!fg) return null;
              const bg = resolveBackground(el);
              const ratio = contrastRatio({ ...fg, a: 1 }, bg);
              const fontSize = Number.parseFloat(style.fontSize || '16');
              const fontWeight = Number.parseInt(style.fontWeight || '400', 10);
              const isLarge = fontSize >= 24 || (fontSize >= 18.66 && fontWeight >= 700);
              const threshold = isLarge ? 3.0 : 4.5;
              if (ratio >= threshold) return null;
              const className = (el instanceof HTMLElement ? el.className : '').toString().replace(/\s+/g, '.').slice(0, 50);
              return `${el.tagName.toLowerCase()}.${className || 'no-class'}(${ratio.toFixed(2)}) fg:${style.color} bg:${getComputedStyle(el.parentElement ?? document.body).backgroundColor}`;
            })
            .filter((value): value is string => Boolean(value))
            .slice(0, 12);

          const rootStyle = getComputedStyle(document.documentElement);
          const tokenPairs = [
            ['--foreground', '--background'],
            ['--card-foreground', '--card'],
            ['--muted-foreground', '--muted'],
          ] as const;

          const tokenFailures = tokenPairs
            .map(([fgName, bgName]) => {
              const fgRaw = rootStyle.getPropertyValue(fgName).trim();
              const bgRaw = rootStyle.getPropertyValue(bgName).trim();
              const fg = parseColor(`rgb(${fgRaw.replace(/\s+/g, ',')})`);
              const bg = parseColor(`rgb(${bgRaw.replace(/\s+/g, ',')})`);
              if (!fg || !bg) return `${fgName}/${bgName}:unresolved`;
              const ratio = contrastRatio(fg, bg);
              const min = fgName === '--muted-foreground' ? 3.0 : 4.5;
              return ratio < min ? `${fgName}/${bgName}(${ratio.toFixed(2)})` : null;
            })
            .filter((value): value is string => Boolean(value));

          const weakBorders = Array.from(document.querySelectorAll('.apex-card, header, aside'))
            .filter((el) => {
              const style = getComputedStyle(el);
              const borderColor = parseColor(style.borderColor);
              const bg = resolveBackground(el);
              if (!borderColor) return false;
              return contrastRatio({ ...borderColor, a: 1 }, bg) < 1.25;
            })
            .slice(0, 8)
            .map((el) => el.tagName.toLowerCase());

          return { lowContrast, tokenFailures, weakBorders };
        });

        if (runtimeErrors.length) {
          findings.push(`${profile.name} ${route} runtime-errors: ${runtimeErrors.slice(0, 2).join(' | ')}`);
        }
        if (metrics.lowContrast.length) {
          findings.push(`${profile.name} ${route} low-contrast:${metrics.lowContrast.join(',')}`);
        }
        if (metrics.tokenFailures.length) {
          findings.push(`${profile.name} ${route} token-contrast:${metrics.tokenFailures.join(',')}`);
        }
        if (metrics.weakBorders.length) {
          findings.push(`${profile.name} ${route} weak-borders:${metrics.weakBorders.join(',')}`);
        }
      }

      await context.close();
    }

    expect(findings, `Dark mode findings:\n${findings.join('\n')}`).toEqual([]);
  });
});

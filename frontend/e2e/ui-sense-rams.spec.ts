import { expect, test, type Page } from '@playwright/test';

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

type ConformanceResult = {
  errors: string[];
  diagnostics: {
    navCount: number;
    activeNavCount: number;
    apexCardCount: number;
    bodyBackgroundImage: string;
    bodyBackgroundColor: string;
  };
};

async function assertSenseiRamsConformance(route: string, page: Page) {
  const result = await page.evaluate((): ConformanceResult => {
    const errors: string[] = [];
    const root = document.documentElement;
    const body = document.body;

    const rootStyle = getComputedStyle(root);
    const bodyStyle = getComputedStyle(body);

    const requiredTokens = [
      '--rams-chassis',
      '--rams-module',
      '--rams-panel',
      '--rams-line',
      '--rams-muted',
      '--rams-orange',
      '--rams-red',
      '--rams-green',
      '--rams-steel',
    ];

    for (const token of requiredTokens) {
      if (!rootStyle.getPropertyValue(token).trim()) {
        errors.push(`missing-token:${token}`);
      }
    }

    if (!bodyStyle.backgroundImage.includes('radial-gradient')) {
      errors.push('missing-chassis-texture:body background is not radial-gradient based');
    }

    const shellFrame = document.querySelector('div.fixed.inset-0[aria-hidden="true"]');
    if (!shellFrame) {
      errors.push('missing-shell-frame:global chassis frame not found');
    }

    if (document.querySelector('.backdrop-blur')) {
      errors.push('forbidden-blur:backdrop-blur class detected in shell chrome');
    }

    const header = document.querySelector('main header');
    if (!header) {
      errors.push('missing-header:app header not found');
    }

    const liveBadge = Array.from(document.querySelectorAll('header span')).find((el) =>
      (el.textContent ?? '').trim().toLowerCase() === 'live',
    );
    if (!liveBadge) {
      errors.push('missing-live-badge:header live status chip is absent');
    }

    const navItems = Array.from(document.querySelectorAll('aside nav a.sidebar-item'));
    const activeNavItems = navItems.filter((el) => el.classList.contains('active'));
    if (navItems.length !== 11) {
      errors.push(`nav-item-count:expected=11 actual=${navItems.length}`);
    }
    if (activeNavItems.length !== 1) {
      errors.push(`active-nav-count:expected=1 actual=${activeNavItems.length}`);
    }

    const apexCards = Array.from(document.querySelectorAll('.apex-card'));
    if (apexCards.length === 0) {
      errors.push('missing-apex-cards:no .apex-card elements found');
    }

    const badCard = apexCards.find((card) => {
      const style = getComputedStyle(card);
      return style.borderRadius !== '2px' || style.boxShadow === 'none';
    });
    if (badCard) {
      errors.push('apex-card-style:expected rounded-sm(2px) and inset chassis shadow');
    }

    const heading = document.querySelector('h2, h3');
    if (heading) {
      const headingStyle = getComputedStyle(heading);
      if (headingStyle.textTransform !== 'uppercase') {
        errors.push(`heading-transform:expected=uppercase actual=${headingStyle.textTransform}`);
      }
    } else {
      errors.push('missing-heading:no h2/h3 heading found');
    }

    const documentMarkup = document.documentElement.outerHTML;
    const forbiddenHexLiterals = ['#F97316', '#FF6B35', '#9B59B6'];
    for (const literal of forbiddenHexLiterals) {
      if (documentMarkup.includes(literal)) {
        errors.push(`forbidden-hex:${literal}`);
      }
    }

    return {
      errors,
      diagnostics: {
        navCount: navItems.length,
        activeNavCount: activeNavItems.length,
        apexCardCount: apexCards.length,
        bodyBackgroundImage: bodyStyle.backgroundImage,
        bodyBackgroundColor: bodyStyle.backgroundColor,
      },
    };
  });

  expect(
    result.errors,
    `Sensei-Rams conformance failed for route ${route}\nDiagnostics: ${JSON.stringify(result.diagnostics, null, 2)}`,
  ).toEqual([]);
}

test.describe('Sense-Rams visual and runtime audit', () => {
  for (const route of routes) {
    test(`route ${route} should render without runtime/server errors and match visual baseline`, async ({ page }) => {
      const errors: string[] = [];

      page.on('pageerror', (error) => {
        errors.push(`pageerror:${error.message}`);
      });

      page.on('console', (message) => {
        if (message.type() === 'error') {
          const text = message.text();
          // Ignore known non-critical errors during testing
          if (!/hydration|did not match|favicon\.ico|Failed to load resource|fetch|network|ECONNREFUSED|api\/health|api\/endpoints|Warning.*Error.*Boundary/i.test(text)) {
            errors.push(`console:${text}`);
          }
        }
      });

      page.on('response', (response) => {
        if (response.status() >= 500) {
          errors.push(`http:${response.status()} ${response.url()}`);
        }
      });

      await page.goto(route, { waitUntil: 'domcontentloaded' });
      // Wait for main content to stabilise (hydration + data queries)
      await page.waitForSelector('.apex-card', { timeout: 15_000 });
      // Ensure any React Query fetches have settled
      await page.waitForTimeout(500);
      await page.addStyleTag({
        content: '*,:before,:after{animation:none!important;transition:none!important;}',
      });

      const textContent = (await page.locator('body').innerText()).slice(0, 5000);
      if (/\b500\b|Internal Server Error/i.test(textContent)) {
        errors.push('ui:Detected 500/Internal Server Error text in page body');
      }

      expect(errors, `Runtime or server errors found for route ${route}`).toEqual([]);
      await assertSenseiRamsConformance(route, page);
      await expect(page).toHaveScreenshot(`sense-rams-${route === '/' ? 'overview' : route.slice(1)}.png`, {
        fullPage: true,
        maxDiffPixelRatio: 0.02,
      });
    });
  }
});

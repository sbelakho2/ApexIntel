const { test, expect } = require('@playwright/test');

async function login(page) {
  const username = process.env.PLAYWRIGHT_USER;
  const password = process.env.PLAYWRIGHT_PASSWORD;
  test.skip(!username || !password, 'PLAYWRIGHT_USER and PLAYWRIGHT_PASSWORD are required for authenticated UI coverage');

  await page.goto('/login');
  await page.getByLabel(/operator id/i).fill(username);
  await page.getByLabel(/access key/i).fill(password);
  await page.locator('#login-submit').click();
}

test('login page visual baseline', async ({ page }) => {
  await page.goto('/login');
  await expect(page).toHaveScreenshot('login-page.png', { fullPage: true });
});

test('dashboard visual baseline', async ({ page }) => {
  await login(page);
  await page.goto('/');
  await expect(page).toHaveScreenshot('dashboard-page.png', { fullPage: true });
});

test('warnings workflow visual baseline', async ({ page }) => {
  await login(page);
  await page.goto('/warnings');
  await expect(page).toHaveScreenshot('warnings-list.png', { fullPage: true });
});

test('graph explorer visual baseline', async ({ page }) => {
  await login(page);
  await page.goto('/graph');
  await page.locator('#graph-container svg').waitFor({ state: 'attached' });
  await expect(page).toHaveScreenshot('graph-page.png', { fullPage: true });
});
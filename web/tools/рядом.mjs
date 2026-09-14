import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1300, height: 700 }, colorScheme: 'dark' });
const p = await ctx.newPage();
await p.goto('file://' + process.env.F, { waitUntil: 'networkidle' });
await p.waitForTimeout(700);
await p.locator('.shot').first().screenshot({ path: process.env.SHOT + '-макет-доска.png' });
await p.locator('.shot').nth(2).screenshot({ path: process.env.SHOT + '-макет-этапы.png' });
await b.close();

import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 940 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
const p = await ctx.newPage();
await p.goto(`http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus&lang=${process.env.L ?? 'ru'}`, { waitUntil: 'networkidle' });
await p.waitForTimeout(1700);
if (process.env.KIND) { await p.locator('.co-kinds button').filter({ hasText: process.env.KIND }).first().click(); await p.waitForTimeout(1200); }
console.log('  колонки:', (await p.locator('.co-tbl th').allInnerTexts()).join(' · '));
console.log('  первая строка:', (await p.locator('.co-tbl tbody tr').first().innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,90));
await p.screenshot({ path: process.env.SHOT });
await b.close();

import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.wk-list li button'); await p.waitForTimeout(1600);
// зеркало проверок — строка с именем R-
const зерк = p.locator('.wk-list li button', { hasText: 'R-' }).first();
if (await зерк.count()) {
  await зерк.click(); await p.waitForTimeout(2200);
  const т = (await p.locator('.drawer, [role=dialog]').innerText()).replace(/\s+/g, ' ');
  say('зеркало открывается без отказа', !т.includes('Не читается') && !т.includes('502'), т.slice(0, 80));
  say('тело зеркала показано', т.length > 300, `${т.length} знаков`);
  await p.keyboard.press('Escape'); await p.waitForTimeout(600);
} else console.log('  ◌ зеркало в очереди — ни одного R-, проверять нечем');
const обыч = p.locator('.wk-list li button').filter({ hasNotText: 'R-' }).first();
await обыч.click(); await p.waitForTimeout(2200);
const т2 = (await p.locator('.drawer, [role=dialog]').innerText()).replace(/\s+/g, ' ');
say('обычная задача открывается', !т2.includes('Не читается'), т2.slice(0, 70));
say('ссылки в дровере есть', (await p.locator('.drawer a.go, [role=dialog] a.go').count()) > 0,
    String(await p.locator('.drawer a.go, [role=dialog] a.go').count()));
await b.close();

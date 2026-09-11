import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [name, w, lang] of [['стол · РУ', 1440, 'ru'], ['стол · EN', 1440, 'en'], ['телефон', 390, 'ru']]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 1000 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
  const p = await ctx.newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));
  await p.goto(`http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=depends&lang=${lang}`, { waitUntil: 'networkidle' });
  await p.waitForTimeout(1600);
  console.log(`══ ${name}`);
  const dom = await p.locator('.dep-row.dom').count();
  say('карта областей', dom === 8, `${dom} рёбер`);
  say('первое ребро', true, (await p.locator('.dep-row.dom').first().innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,52));
  await p.locator('.dep-fold button').nth(1).click(); await p.waitForTimeout(600);
  const kin = await p.locator('.dep-row:not(.dom)').count();
  say('карта родов', kin === 7, `${kin} строк`);
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.locator('.dep-fold button').first().click(); await p.waitForTimeout(500);
  await p.screenshot({ path: process.env.SHOT.replace('.png', `-${w}-${lang}.png`) });
  await ctx.close();
}
await b.close();

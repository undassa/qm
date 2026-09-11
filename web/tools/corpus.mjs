import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [name, w, h, lang] of [['стол · РУ', 1440, 1000, 'ru'], ['телефон · РУ', 390, 844, 'ru'], ['стол · EN', 1440, 1000, 'en']]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
  const p = await ctx.newPage();
  const errs = [];
  p.on('pageerror', e => errs.push(String(e).slice(0, 90)));
  await p.goto(`http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus&lang=${lang}`, { waitUntil: 'networkidle' });
  await p.waitForTimeout(1700);
  console.log(`══ ${name}`);
  say('родов в списке', (await p.locator('.co-kinds li').count()) > 8, String(await p.locator('.co-kinds li').count()));
  say('строк в таблице', (await p.locator('.co-tbl tbody tr').count()) > 10, String(await p.locator('.co-tbl tbody tr').count()));
  const nav = await p.locator('.side .ent.top .ent-n').allInnerTexts();
  say('разделов', nav.length === 7, nav.join(' · '));
  // Клик по строке — раскладка одной записи.
  await p.locator('.co-tbl tbody tr').first().click();
  await p.waitForTimeout(1100);
  say('раскладка открылась', (await p.locator('.co-one').count()) === 1, (await p.locator('.co-one-id').innerText().catch(()=>'')));
  say('ячеек раскладки', (await p.locator('.co-cell').count()) > 2, String(await p.locator('.co-cell').count()));
  await p.locator('.co-back').click(); await p.waitForTimeout(500);
  say('возврат к таблице', (await p.locator('.co-tbl').count()) === 1);
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.screenshot({ path: process.env.SHOT.replace('.png', `-${w}-${lang}.png`) });
  await ctx.close();
}
await b.close();

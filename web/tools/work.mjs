import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [name, w] of [['стол', 1440], ['телефон', 390]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 1000 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
  const p = await ctx.newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));
  await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=tasks', { waitUntil: 'networkidle' });
  await p.waitForTimeout(1700);
  console.log(`══ ${name}`);
  say('сводка', (await p.locator('.wk-say').innerText().catch(()=>'')).length > 20, (await p.locator('.wk-say').innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,64));
  say('волн', (await p.locator('.wk-tl li').count()) > 2, String(await p.locator('.wk-tl li').count()));
  say('задач', (await p.locator('.wk-list li').count()) > 10, String(await p.locator('.wk-list li').count()));
  await p.locator('.wk-lane').first().click(); await p.waitForTimeout(500);
  say('волна отбирает', (await p.locator('.wk-list li').count()) > 0, String(await p.locator('.wk-list li').count()) + ' после отбора');
  await p.locator('.wk-list button').first().click(); await p.waitForTimeout(1400);
  say('дровер открылся', (await p.locator('.drawer, [role=dialog]').count()) > 0);
  await p.keyboard.press('Escape'); await p.waitForTimeout(400);
  say('Escape закрывает', (await p.locator('.drawer, [role=dialog]').count()) === 0);
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.screenshot({ path: process.env.SHOT.replace('.png', `-${w}.png`) });
  await ctx.close();
}
await b.close();

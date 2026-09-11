import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [name, proj, w, h] of [
  ['стол · myack', '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3', 1440, 1000],
  ['телефон · myack', '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3', 390, 844],
  ['стол · tot-ade', 'ae7ec7fa-62d3-4664-89d2-18f35a25eb3a', 1440, 1000],
]) {
  const ctx = await b.newContext({ viewport: { width: w, height: h } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
  const p = await ctx.newPage();
  const errs = [];
  p.on('pageerror', e => errs.push(String(e).slice(0, 90)));
  await p.goto(`http://127.0.0.1:8096/next/?project=${proj}`, { waitUntil: 'networkidle' });
  await p.waitForTimeout(1600);
  console.log(`══ ${name}`);
  const says = await p.locator('.pu-say').innerText().catch(() => '');
  say('фраза состояния', says.length > 10, says.replace(/\s+/g, ' ').slice(0, 72));
  say('шкала ступеней', (await p.locator('.pu-rung i').count()) === 13, String(await p.locator('.pu-rung i').count()));
  say('полос', (await p.locator('.pu-band').count()) === 3, String(await p.locator('.pu-band').count()));
  say('ждут', true, (await p.locator('.pu-h').nth(1).innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,44));
  say('поехало', true, (await p.locator('.pu-h').nth(2).innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,44));
  const doc = await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1);
  say('страница не едет вбок', doc);
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.screenshot({ path: process.env.SHOT.replace('.png', `-${w}-${proj.slice(0,4)}.png`) });
  await ctx.close();
}
await b.close();

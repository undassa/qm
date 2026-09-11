import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [name, w, kind, by] of [
  ['стол · требование по области', 1440, null, 'область'],
  ['стол · вопрос', 1440, 'вопрос', null],
  ['телефон · требование', 390, null, null],
]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 1000 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
  const p = await ctx.newPage();
  const errs = [];
  p.on('pageerror', e => errs.push(String(e).slice(0, 90)));
  await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus', { waitUntil: 'networkidle' });
  await p.waitForTimeout(1600);
  if (kind) { await p.locator('.co-kinds button').filter({ hasText: kind }).first().click(); await p.waitForTimeout(1200); }
  if (by) { await p.locator('.co-by button').filter({ hasText: by }).first().click(); await p.waitForTimeout(700); }
  console.log(`══ ${name}`);
  const g = await p.locator('.co-grp th span').allInnerTexts();
  say('групп', g.length > 0, g.slice(0, 6).join(' · ') + (g.length > 6 ? ` … всего ${g.length}` : ''));
  const c = await p.locator('.co-grp th b').allInnerTexts();
  say('счёт в заголовке', c.length === g.length, c.slice(0, 6).join(' · '));
  say('строк', (await p.locator('.co-tbl tbody tr:not(.co-grp)').count()) > 5, String(await p.locator('.co-tbl tbody tr:not(.co-grp)').count()));
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.screenshot({ path: process.env.SHOT.replace('.png', `-${w}-${kind ?? by ?? 'def'}.png`) });
  await ctx.close();
}
await b.close();

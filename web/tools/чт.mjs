import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [имя, w] of [['телефон', 390], ['стол', 1440]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 900 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
  const p = await ctx.newPage();
  await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=read', { waitUntil: 'domcontentloaded' });
  await p.waitForSelector('.reader-doc'); await p.waitForTimeout(1400);
  console.log(`══ ${имя} (${w})`);
  const свёртка = await p.locator('.rd-fold').isVisible();
  say(w < 861 ? 'свёртка видов есть' : 'свёртки нет', w < 861 ? свёртка : !свёртка);
  const док = await p.locator('.reader-doc').boundingBox();
  say('документ шире 300', док.width > 300, `${Math.round(док.width)}px`);
  const абз = await p.locator('.reader-doc p').first();
  const т = (await абз.innerText()).split('\n')[0] ?? '';
  say('строка длиннее 30 знаков', т.length > 30, `${т.length} знаков: ${т.slice(0,48)}`);
  if (w < 861) {
    await p.locator('.rd-fold').click(); await p.waitForTimeout(400);
    const видов = await p.locator('.reader-kinds .rk:visible').count();
    say('виды раскрылись лентой', видов > 10, `${видов} видов`);
    const ряды = new Set(await p.locator('.reader-kinds .rk').evaluateAll(es => es.map(e => Math.round(e.getBoundingClientRect().top))));
    say('виды переносятся, а не в столбик', ряды.size < видов, `${ряды.size} строк на ${видов} видов`);
    await p.locator('.reader-kinds .rk', { hasText: 'decision' }).first().click(); await p.waitForTimeout(1200);
    say('выбор вида схлопнул ленту', !(await p.locator('.reader-kinds').isVisible()));
    say('список имён появился', (await p.locator('.rk-list .rk').count()) > 5, String(await p.locator('.rk-list .rk').count()));
  }
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  await p.screenshot({ path: `${process.env.SHOT}-чт-${w}.png` });
  await ctx.close();
}
await b.close();

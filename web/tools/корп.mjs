import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
for (const [имя, w] of [['телефон', 390], ['стол', 1440]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 900 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
  const p = await ctx.newPage();
  await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus', { waitUntil: 'domcontentloaded' });
  await p.waitForSelector('.co-tbl tbody tr'); await p.waitForTimeout(1200);
  console.log(`══ ${имя} (${w})`);
  const свёрткаВидна = await p.locator('.co-fold').isVisible();
  say(w < 620 ? 'свёртка есть' : 'свёртки нет', w < 620 ? свёрткаВидна : !свёрткаВидна);
  const доТаблицы = Math.round((await p.locator('.co-tbl').boundingBox()).y);
  say('таблица близко к верху', w >= 620 || доТаблицы < 420, `${доТаблицы}px`);
  if (w < 620) {
    const стр = await p.locator('.co-tbl tbody tr').nth(1);
    const т = (await стр.innerText()).replace(/\s+/g,' ');
    say('в строке видно имя и заголовок целиком', т.length > 30 && !т.includes('…'), т.slice(0, 80));
    await p.locator('.co-fold').click(); await p.waitForTimeout(400);
    say('свёртка раскрывается', (await p.locator('.co-kinds section').first().isVisible()));
    await p.locator('.co-kinds button.on, .co-kinds li button').first().click(); await p.waitForTimeout(500);
    say('выбор рода схлопывает список', !(await p.locator('.co-kinds section').first().isVisible()));
  }
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  await p.screenshot({ path: `${process.env.SHOT}-корп-${w}.png` });
  await ctx.close();
}
await b.close();

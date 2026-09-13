import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const errs = [];

for (const [имя, w] of [['стол', 1440], ['телефон', 390]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 950 } });
  ctx.setDefaultTimeout(20000);
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
  const p = await ctx.newPage();
  p.on('pageerror', e => errs.push(String(e).slice(0, 80)));
  console.log(`══ ${имя} (${w})`);
  await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
  await p.waitForSelector('.wk-list li', { state: 'attached' }); await p.waitForTimeout(1600);

  // Поиск виден без клавиатуры
  const кн = w < 861 ? '.find' : '.side-find';
  say('кнопка поиска видна', await p.locator(кн).isVisible(), кн);
  await p.locator(кн).click(); await p.waitForTimeout(900);
  say('палитра открылась', await p.locator('.pal').isVisible());
  // Ловушка фокуса: десять Tab не выводят наружу
  for (let i = 0; i < 10; i++) await p.keyboard.press('Tab');
  say('фокус держится в палитре', await p.evaluate(() => !!document.querySelector('.pal')?.contains(document.activeElement)),
      await p.evaluate(() => document.activeElement?.className || document.activeElement?.tagName || '?'));
  await p.keyboard.press('Escape'); await p.waitForTimeout(500);
  say('Escape закрыл палитру', !(await p.locator('.pal').isVisible()));

  // Зеркало помечено
  const зерк = await p.locator('.wk-list li', { hasText: 'R-' }).first();
  if (await зерк.count()) say('зеркало помечено значком', (await зерк.locator('.wk-pair').count()) === 1);
  else say('зеркало в очереди', false, 'ни одного R-');
  const обыч = p.locator('.wk-list li').filter({ hasNotText: 'R-' }).first();
  say('обычной задаче значок не ставится', (await обыч.locator('.wk-pair').count()) === 0);

  if (w < 861) {
    await p.locator('.burger').click(); await p.waitForTimeout(500);
    for (let i = 0; i < 12; i++) await p.keyboard.press('Tab');
    say('фокус держится в ящике', await p.evaluate(() => !!document.querySelector('.side')?.contains(document.activeElement)),
        await p.evaluate(() => document.activeElement?.textContent?.trim().slice(0, 24) || '?'));
    await p.keyboard.press('Escape'); await p.waitForTimeout(400);
  }
  await ctx.close();
}

// Исходящие ссылки в шапке документа
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
p.on('pageerror', e => errs.push(String(e).slice(0, 80)));
console.log('══ исходящие ссылки документа');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-0138`, { waitUntil: 'domcontentloaded' });
await p.waitForFunction(() => (document.querySelector('.doc-head')?.textContent ?? '').includes('ADR-0138'), null, { timeout: 25000 });
await p.waitForTimeout(2000);
const ведёт = await p.locator('.backs').first();
const т = (await ведёт.innerText()).replace(/\s+/g, ' ');
say('блок «Ведёт на» есть до раскрытия разделов', /Ведёт на|Points to/.test(т), т.slice(0, 70));
const шт = await p.locator('.backs').first().locator('a.go').count();
say('ссылки в нём кликабельны', шт > 0, `${шт} штук`);
say('без ошибок', errs.length === 0, errs[0] ?? '');
await b.close();

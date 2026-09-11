import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const U = 'http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=tasks';
for (const [name, w] of [['стол', 1440], ['телефон', 390]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 1200 } });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
  const p = await ctx.newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 120)));
  await p.goto(U, { waitUntil: 'networkidle' });
  await p.waitForTimeout(1800);
  console.log(`══ ${name} (${w})`);
  say('переключатель', (await p.locator('.wk-views button').count()) >= 3, String(await p.locator('.wk-views button').count()));
  const шапка = (await p.locator('.wk-say').first().innerText().catch(()=>''))
  say('шапка', шапка.length > 10, шапка.replace(/\s+/g,' ').slice(0,80));

  // ── доска
  await p.locator('.wk-views button').nth(1).click(); await p.waitForTimeout(900);
  const кол = await p.locator('.wk-col').count();
  say('колонок доски', кол === 6, String(кол));
  const заг = await p.locator('.wk-colh').allInnerTexts();
  console.log('    ' + заг.map(t=>t.replace(/\s+/g,' ')).join(' | ').slice(0,190));
  const карт = await p.locator('.wk-cards li').count();
  say('карточек видно', карт > 5, String(карт));
  const рв = await p.locator('.wk-cards li.torn').count();
  say('рваных помечено', рв > 0, String(рв));
  const тр = await p.locator('.wk-cards li.torn .wk-torn').count();
  say('значок пропуска', тр > 0, String(тр));
  const бт = await p.locator('.wk-bt').first().boundingBox();
  say('название не в труху', (бт?.width ?? 0) > 120, `${Math.round(бт?.width ?? 0)}px ширина, ${Math.round(бт?.height ?? 0)}px высота`);
  const пв = await p.locator('.wk-cards li').first().innerText().catch(()=>'');
  say('в карточке видно имя и название', пв.split('\n').filter(Boolean).length >= 2, пв.replace(/\s+/g,' ').slice(0,70));
  const край = await p.evaluate(() => {
    const d = document.querySelector('.wk-board'); const c = d.lastElementChild;
    return { need: d.scrollWidth, have: d.clientWidth, last: c.getBoundingClientRect().right, win: window.innerWidth };
  });
  if (w >= 860) {
    say('все шесть колонок помещаются', край.need <= край.have + 1 && край.last <= край.win + 1,
        `нужно ${Math.round(край.need)}, есть ${край.have}; край ${Math.round(край.last)} при окне ${край.win}`);
  } else {
    say('доска едет вбок внутри себя', край.need > край.have + 1,
        `нужно ${Math.round(край.need)}, есть ${край.have}`);
  }
  await p.screenshot({ path: process.env.SHOT + `-доска-${w}.png`, fullPage: w === 390 });

  // ── этапы
  await p.locator('.wk-views button').nth(2).click(); await p.waitForTimeout(900);
  const эт = await p.locator('.wk-tl li').count();
  say('этапов', эт >= 8, String(эт));
  say('легенда', (await p.locator('.wk-key li').count()) === 6, String(await p.locator('.wk-key li').count()));
  const сег = await p.locator('.wk-track i').count();
  say('сегментов полос', сег > 8, String(сег));
  await p.screenshot({ path: process.env.SHOT + `-этапы-${w}.png` });

  // этап уводит на доску отбором
  await p.locator('.wk-lane').first().click(); await p.waitForTimeout(800);
  say('этап уводит на доску', (await p.locator('.wk-col').count()) === 6);
  const после = await p.locator('.wk-cards li').count();
  say('отбор сузил доску', после > 0 && после < карт, `${после} из ${карт}`);
  await p.locator('.wk-all').click().catch(()=>{}); await p.waitForTimeout(500);

  // ── дровер с доски
  await p.locator('.wk-cards button').first().click(); await p.waitForTimeout(1500);
  say('дровер открылся', (await p.locator('.drawer, [role=dialog]').count()) > 0);
  await p.keyboard.press('Escape'); await p.waitForTimeout(400);

  // ── список
  await p.locator('.wk-views button').nth(0).click(); await p.waitForTimeout(800);
  say('список вернулся', (await p.locator('.wk-list li').count()) > 10, String(await p.locator('.wk-list li').count()));
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await ctx.close();
}
await b.close();

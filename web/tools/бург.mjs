import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
for (const [имя, w] of [['телефон', 390], ['стол', 1440]]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 844 }, hasTouch: w < 860 });
  await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
  const p = await ctx.newPage();
  const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0,90)));
  await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
  await p.waitForSelector('.ent.top', { state: 'attached' }); await p.waitForTimeout(1300);
  console.log(`══ ${имя} (${w})`);
  const планка = await p.locator('.topbar').isVisible();
  say(w < 861 ? 'планка есть' : 'планки нет', w < 861 ? планка : !планка);
  if (w >= 861) {
    say('боковик на месте', (await p.locator('.side').isVisible()) && (await p.locator('.side').boundingBox()).width > 200);
    say('разделы видны без нажатий', (await p.locator('.ent.top:visible').count()) === 7);
    await ctx.close(); continue;
  }
  const б = await p.locator('.burger').boundingBox();
  say('кнопка не меньше 44×44', б.width >= 44 && б.height >= 44, `${Math.round(б.width)}×${Math.round(б.height)}`);
  say('имя раздела на планке', (await p.locator('.topbar-t').innerText()).trim().length > 2, await p.locator('.topbar-t').innerText());
  say('ящик закрыт', !(await p.locator('.side').isVisible()));
  const дано = await p.evaluate(() => {
    const m = document.querySelector('.main').getBoundingClientRect();
    return { верх: Math.round(m.top), высота: Math.round(m.height) };
  });
  say('содержимому отдано почти всё', дано.верх <= 50, `раздел начинается с ${дано.верх}px`);
  say('закрытый ящик не ловится табом', await p.evaluate(() =>
    getComputedStyle(document.querySelector('.side')).visibility === 'hidden'));

  await p.locator('.burger').click(); await p.waitForTimeout(400);
  say('ящик открылся', await p.locator('.side').isVisible());
  say('затемнение появилось', await p.locator('.scrim').isVisible());
  say('все семь разделов видны', (await p.locator('.ent.top:visible').count()) === 7);
  say('пояснения вернулись', (await p.locator('.ent.top .ent-c:visible').count()) === 7);
  const пункт = await p.locator('.ent.top').first().boundingBox();
  say('пункт не ниже 44', пункт.height >= 44, `${Math.round(пункт.height)}px`);
  const я = await p.locator('.side').boundingBox();
  const яз = await p.locator('.lang').boundingBox();
  say('язык в подвале ящика', яз.y > я.y + я.height * 0.6, `язык на ${Math.round(яз.y)}, ящик до ${Math.round(я.y + я.height)}`);
  const зат = await p.evaluate(() => getComputedStyle(document.querySelector('.scrim')).backgroundColor);
  say('затемнение непрозрачно', /0\.[3-9]/.test(зат) || /rgb\(/.test(зат), зат);
  say('aria-expanded=true', (await p.locator('.burger').getAttribute('aria-expanded')) === 'true');

  await p.keyboard.press('Escape'); await p.waitForTimeout(400);
  say('Escape закрывает', !(await p.locator('.side').isVisible()));

  await p.locator('.burger').click(); await p.waitForTimeout(350);
  await p.locator('.scrim').click({ position: { x: 370, y: 700 } }); await p.waitForTimeout(400);
  say('нажатие мимо закрывает', !(await p.locator('.side').isVisible()));

  await p.locator('.burger').click(); await p.waitForTimeout(350);
  await p.locator('.ent.top', { hasText: 'Корпус' }).first().click(); await p.waitForTimeout(800);
  say('выбор раздела закрывает ящик', !(await p.locator('.side').isVisible()));
  say('раздел сменился', (await p.locator('.topbar-t').innerText()).includes('Корпус'), await p.locator('.topbar-t').innerText());
  say('страница не едет вбок', await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1));
  say('без ошибок', errs.length === 0, errs[0] ?? '');
  await p.screenshot({ path: `${process.env.SHOT}-закрыт.png` });
  await p.locator('.burger').click(); await p.waitForTimeout(400);
  await p.screenshot({ path: `${process.env.SHOT}-открыт.png` });
  await ctx.close();
}
await b.close();

import { chromium } from 'playwright';
const b = await chromium.launch();
for (const w of [1280, 390]) {
  const ctx = await b.newContext({ viewport: { width: w, height: 1000 }, colorScheme: 'dark' });
  const p = await ctx.newPage();
  await p.goto('file://' + process.env.F, { waitUntil: 'networkidle' });
  await p.waitForTimeout(800);
  const bd = await p.locator('.bd').boundingBox();
  console.log(`══ ${w}`);
  console.log(`  доска начинается на ${Math.round(bd.y)}px от верха · видна в первом экране: ${bd.y < 1000}`);
  console.log(`  колонок ${await p.locator('.bd-col').count()} · карточек ${await p.locator('.bd-card').count()}`);
  const ид = await p.locator('.bd-card u').allInnerTexts();
  const дубли = ид.filter((x, i) => ид.indexOf(x) !== i);
  console.log(`  повторов имён: ${дубли.length} ${дубли.join(',')}`);
  const к = await p.locator('.cap .key').first().boundingBox();
  console.log(`  число «16 через голову» видно без прокрутки: ${к && к.x >= 0 && к.x + к.width <= w + 1}`);
  const зн = await p.locator('.bd-card.torn').first().locator('s').boundingBox();
  const ид2 = await p.locator('.bd-card.torn').first().locator('u').boundingBox();
  console.log(`  знак ↯ отъехал вправо: ${зн && ид2 ? Math.round(зн.x - ид2.x - ид2.width) + 'px зазор' : 'НЕТ'}`);
  console.log(`  страница не едет вбок: ${await p.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth + 1)}`);
  await p.screenshot({ path: process.env.SHOT + `-арт-${w}.png` });
  await ctx.close();
}
await b.close();

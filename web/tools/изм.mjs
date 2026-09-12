import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 390, height: 844 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=tasks', { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.ent.top'); await p.waitForTimeout(1200);
console.log(await p.evaluate(() => {
  const пилюли = [...document.querySelectorAll('.ent.top')].map(e => ({
    имя: e.textContent.trim(), ш: Math.round(e.getBoundingClientRect().width), в: Math.round(e.getBoundingClientRect().height) }));
  const side = document.querySelector('.side');
  const поле = side.clientWidth - 24;
  const сумма = пилюли.reduce((a, x) => a + x.ш, 0) + (пилюли.length - 1) * 6;
  return { поле, сумма, надоСтрок: Math.ceil(сумма / поле), пилюли,
           карта: Math.round(document.querySelector('.picker').getBoundingClientRect().width),
           язык: Math.round(document.querySelector('.lang').getBoundingClientRect().width) };
}));
await b.close();

import { chromium } from 'playwright';
const b = await chromium.launch();
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 1100 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.wk-views button'); await p.waitForTimeout(1800);

const ч = (s) => s.replace(/\s+/g, ' ').trim();
console.log('══ СПИСОК (что сейчас)');
console.log('  шапка страницы:', ч(await p.locator('.wk-say').first().innerText()));
console.log('  волны:', ч(await p.locator('.wk-h').first().innerText()));
console.log('  шапка таблицы:', ч(await p.locator('.wk-thead').innerText()));
console.log('  первая строка:', ч(await p.locator('.wk-list li').first().innerText()));
const низ = await p.locator('.wk-gap').allInnerTexts();
console.log('  строки снизу:', низ.map(ч).map(t => t.slice(0, 78)));

console.log('══ ДОСКА');
await p.locator('.wk-views button').nth(1).click(); await p.waitForTimeout(1200);
console.log('  подпись сверху:', ч(await p.locator('.wk-note').first().innerText()));
console.log('  колонки:', (await p.locator('.wk-colh').allInnerTexts()).map(ч).join(' | '));
const хвосты = await p.locator('.wk-col .wk-why').evaluateAll(es => es.map(e => Math.round(e.getBoundingClientRect().top)));
  const заполненные = хвосты.slice(0, 4);
  console.log('  «ещё N» вровень:', new Set(заполненные).size === 1, заполненные.join('/'));
  console.log('  высота доски:', Math.round((await p.locator('.wk-board').boundingBox()).height) + 'px');
console.log('  карточек в колонках:', (await p.locator('.wk-col').evaluateAll(cs => cs.map(c => c.querySelectorAll('.wk-cards li').length))).join('/'));
console.log('  карточка:', ч(await p.locator('.wk-cards li').first().innerText()));
console.log('  снизу:', (await p.locator('.wk-gap').allInnerTexts()).map(ч).map(t => t.slice(0, 88)));

console.log('══ ЭТАПЫ');
await p.locator('.wk-views button').nth(2).click(); await p.waitForTimeout(1200);
console.log('  подпись:', ч(await p.locator('.wk-note').first().innerText()));
console.log('  строки:', (await p.locator('.wk-tl li').allInnerTexts()).map(ч).slice(0, 4).join(' | '));
console.log('  легенда:', (await p.locator('.wk-key li').allInnerTexts()).map(ч).join(' · '));
console.log('  снизу:', (await p.locator('.wk-gap').allInnerTexts()).map(ч).map(t => t.slice(0, 88)));
await b.close();

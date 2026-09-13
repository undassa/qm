import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();

console.log('══ отказ двери показывается её словами');
// Заведомо несуществующее имя: дверь объясняет отказ и называет, чем закрыть.
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-9999`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.empty, .doc-head'); await p.waitForTimeout(2500);
const т = (await p.locator('.reader-doc, .main .empty').first().innerText().catch(async () => await p.locator('.main').innerText())).replace(/\s+/g, ' ');
say('кода и пути на экране нет', !/502|\/api\/projects/.test(т), т.slice(0, 90));
say('слова двери на месте', /нет такой сущности|declared-unwritten/.test(т), т.slice(0, 110));

console.log('══ сервера нет вовсе');
await p.route('**/api/**', r => r.abort());
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=where`, { waitUntil: 'domcontentloaded' }).catch(()=>{});
await p.waitForTimeout(2500);
const т2 = (await p.locator('.main').innerText().catch(()=>'')).replace(/\s+/g, ' ');
say('сказано человеческими словами', /сети нет|не запущен/.test(т2), т2.slice(0, 90));
say('«Error:» и «Failed to fetch» не показаны', !/Error:|Failed to fetch|TypeError/.test(т2), т2.slice(0, 70));
await b.close();

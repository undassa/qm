import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
// Документы: решение ADR-0156 заведомо со ссылками
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-0156`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.reader-doc'); await p.waitForTimeout(2000);
console.log('ДОКУМЕНТЫ: a.go =', await p.locator('a.go').count(), '· раскрыто блоков:', await p.locator('.bl-p, .reader-doc p').count());
await p.locator('.doc-sum button, .reader-doc button').first().click().catch(()=>{});
await p.waitForTimeout(1500);
console.log('  после «раскрыть всё»: a.go =', await p.locator('a.go').count());
const т = (await p.locator('.reader-doc').innerText()).replace(/\s+/g,' ');
console.log('  в тексте видно «ADR-»:', /ADR-\d/.test(т), '· длина текста:', т.length);
// Дровер задачи
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.wk-list li button'); await p.waitForTimeout(1500);
await p.locator('.wk-list li button').first().click(); await p.waitForTimeout(2200);
const д = (await p.locator('.drawer, [role=dialog]').innerText().catch(()=>'')).replace(/\s+/g,' ');
console.log('ДРОВЕР: длина текста', д.length, '· a.go =', await p.locator('a.go').count());
console.log('  начало:', д.slice(0, 200));
await b.close();

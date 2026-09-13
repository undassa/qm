import { chromium } from 'playwright';
const b = await chromium.launch();
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(25000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const плохие = [];
p.on('response', async r => {
  if (!r.url().includes('/tool/blocks')) return;
  if (r.status() !== 200) плохие.push(`${r.status()} ${decodeURIComponent(r.url().split('/tool/')[1]).slice(0, 110)}`);
});
p.on('requestfailed', r => { if (r.url().includes('/tool/blocks')) плохие.push(`СОРВАН ${r.failure()?.errorText} ${decodeURIComponent(r.url().split('/tool/')[1]).slice(0,90)}`); });
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-0138`, { waitUntil: 'domcontentloaded' });
await p.waitForFunction(() => (document.querySelector('.doc-head')?.textContent ?? '').includes('ADR-0138'), null, { timeout: 25000 });
await p.waitForSelector('.doc-secs button');
await p.waitForTimeout(600);
await p.locator('.doc-sum button.ghost').click();
await p.waitForTimeout(4000);
console.log('разделов:', await p.locator('.sec-h').count(), '· раскрыто:', await p.locator('.sec-h.on').count(),
            '· «не прочиталось»:', await p.locator('.side-note.warn').count());
console.log('неудачных запросов блоков:', плохие.length);
for (const x of плохие.slice(0, 6)) console.log('  ', x);
await b.close();

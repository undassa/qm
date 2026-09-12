import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=tasks`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.wk-list li button'); await p.waitForTimeout(1500);
await p.locator('.wk-list li button').first().click();
await p.waitForSelector('.drawer, [role=dialog]'); await p.waitForTimeout(1800);
const n = await p.locator('a.go').count();
console.log('ссылок a.go в дровере:', n);
if (n) {
  const a = p.locator('a.go').first();
  console.log('  первая:', (await a.innerText()).slice(0,40), '→', await a.getAttribute('href'), '| kind:', await a.getAttribute('data-kind'), 'id:', await a.getAttribute('data-id'));
  const былURL = p.url();
  await a.click(); await p.waitForTimeout(1800);
  console.log('  URL до :', былURL.replace('http://127.0.0.1:8096',''));
  console.log('  URL после:', p.url().replace('http://127.0.0.1:8096',''));
  console.log('  проект в адресе:', p.url().includes('project='));
  console.log('  что на экране:', (await p.locator('.main').innerText()).replace(/\s+/g,' ').slice(0,110));
}
console.log('── Корпус: есть ли ссылки в записи');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-tbl tbody tr'); await p.waitForTimeout(1200);
await p.locator('.co-tbl tbody tr').nth(1).click(); await p.waitForTimeout(1500);
console.log('  ссылок a.go в записи корпуса:', await p.locator('a.go').count());
console.log('  текст записи:', (await p.locator('.co-main').innerText()).replace(/\s+/g,' ').slice(0,160));
await b.close();

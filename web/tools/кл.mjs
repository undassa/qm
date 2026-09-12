import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 390, height: 900 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus', { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-tbl tbody tr'); await p.waitForTimeout(1200);
console.log(await p.evaluate(() => {
  const tr = [...document.querySelectorAll('.co-tbl tbody tr')].find(r => r.querySelectorAll('td').length);
  const cs = getComputedStyle(tr);
  return {
    таблица: getComputedStyle(document.querySelector('.co-tbl table')).display,
    tbody: getComputedStyle(tr.parentElement).display,
    tr: cs.display, wrap: cs.flexWrap,
    клетки: [...tr.querySelectorAll('td')].map(td => ({
      k: td.dataset.k, d: getComputedStyle(td).display, order: getComputedStyle(td).order,
      x: Math.round(td.getBoundingClientRect().x), w: Math.round(td.getBoundingClientRect().width),
      t: td.textContent.slice(0, 18) })),
  };
}));
await b.close();

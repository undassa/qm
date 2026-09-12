import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 900 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=corpus', { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-fold'); await p.waitForTimeout(900);
console.log(await p.evaluate(() => {
  const f = document.querySelector('.co-fold');
  const r = f.getBoundingClientRect();
  return { display: getComputedStyle(f).display, w: Math.round(r.width), h: Math.round(r.height),
           правила: [...document.styleSheets].flatMap(ss => { try { return [...ss.cssRules] } catch { return [] } })
             .flatMap(r => r.cssRules ? [...r.cssRules].map(x => (r.conditionText||'') + ' | ' + x.cssText.slice(0,60)) : [r.cssText.slice(0,60)])
             .filter(t => t.includes('.co-fold')) };
}));
await b.close();

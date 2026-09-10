import { chromium } from 'playwright';
const out = process.argv[2], routes = JSON.parse(process.argv[3]), proj = process.argv[4];
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
await ctx.addCookies([{ name: 'mh_identity', value: process.env.MH_TOKEN, url: 'http://127.0.0.1:8096' }]);
const p = await ctx.newPage();
const errs = [];
p.on('pageerror', e => errs.push('PAGEERROR '+String(e).slice(0,120)));
p.on('console', m => { if (m.type()==='error') errs.push(m.text().slice(0,120)); });
for (const [name, path] of routes) {
  errs.length = 0;
  await p.goto(`http://127.0.0.1:8096/next/${path}&project=${proj}`, { waitUntil: 'networkidle', timeout: 25000 });
  await p.waitForTimeout(1100);
  const h = await p.evaluate(() => { const m = document.querySelector('.main'); return m ? m.scrollHeight : 0; });
  await p.setViewportSize({ width: 1440, height: Math.min(Math.max(h + 40, 700), 9000) });
  await p.waitForTimeout(400);
  await p.screenshot({ path: `${out}/${name}.png` });
  console.log(`${name}\t${h}px\t${errs.length} ошибок${errs.length ? ' · ' + errs[0] : ''}`);
  await p.setViewportSize({ width: 1440, height: 950 });
}
await b.close();

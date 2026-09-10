import { chromium } from 'playwright';
const base = 'http://127.0.0.1:8096/next/';
const out = process.argv[2];
const routes = JSON.parse(process.argv[3]);
const wide = Number(process.argv[4] || 1440);
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: wide, height: 950 } });
await ctx.addCookies([{ name: 'mh_identity', value: process.env.MH_TOKEN, url: 'http://127.0.0.1:8096' }]);
const p = await ctx.newPage();
const errs = [];
p.on('console', m => { if (m.type()==='error') errs.push(m.text().slice(0,140)); });
p.on('pageerror', e => errs.push('PAGEERROR '+String(e).slice(0,140)));
for (const [name, path] of routes) {
  errs.length = 0;
  try {
    await p.goto(base + path, { waitUntil: 'networkidle', timeout: 25000 });
    await p.waitForTimeout(1000);
    // Растим окно под ВСЮ внутреннюю прокрутку: каркас — окно в 100vh, и
    // обычный fullPage снимает только первый экран.
    const h = await p.evaluate(() => {
      const m = document.querySelector('.main');
      return m ? Math.min(m.scrollHeight + 40, 12000) : document.body.scrollHeight;
    });
    await p.setViewportSize({ width: wide, height: Math.max(h, 700) });
    await p.waitForTimeout(500);
    await p.screenshot({ path: `${out}/${name}.png` });
    const over = await p.evaluate(() => {
      const bad = [];
      for (const el of document.querySelectorAll('.main *')) {
        if (el.scrollWidth > el.clientWidth + 4 && getComputedStyle(el).overflowX === 'visible')
          bad.push((el.className||el.tagName).toString().slice(0,34));
      }
      return [...new Set(bad)].slice(0, 5);
    });
    console.log(`${name}\t${h}px\t${errs.length} ошибок${over.length ? '\tвылезает: '+over.join(', ') : ''}`);
    if (errs.length) console.log('   ! ' + errs.slice(0,2).join(' ; '));
  } catch (e) { console.log(`${name}\tПАДЕНИЕ\t${String(e).slice(0,110)}`); }
  await p.setViewportSize({ width: wide, height: 950 });
}
await b.close();

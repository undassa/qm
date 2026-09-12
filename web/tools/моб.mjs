import { chromium } from 'playwright';
const b = await chromium.launch();
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const стр = ['pult','where','tasks','unknown','read','corpus','depends'];
const ctx = await b.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
for (const s of стр) {
  const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0,80)));
  await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=${s}`, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await p.waitForSelector('.ent.top', { timeout: 30000 }).catch(()=>{});
  await p.waitForTimeout(2200);
  const d = await p.evaluate(() => {
    const win = window.innerWidth;
    const вЯщике = (e) => {
      for (let n = e.parentElement; n; n = n.parentElement) {
        const o = getComputedStyle(n).overflowX;
        if (o === 'auto' || o === 'scroll') return true;
      }
      return false;
    };
    const шире = [...document.querySelectorAll('*')]
      .filter(e => e.getBoundingClientRect().width > win + 2 && !вЯщике(e))
      .map(e => `${e.tagName.toLowerCase()}.${(e.className||'').toString().split(' ')[0]}=${Math.round(e.getBoundingClientRect().width)}`)
      .slice(0, 5);
    const nav = document.querySelector('nav, .nav, aside');
    const кнопки = [...document.querySelectorAll('button, a')]
      .filter(e => { const r = e.getBoundingClientRect(); return r.height > 0 && r.height < 32; }).length;
    return {
      едетВбок: document.documentElement.scrollWidth > win + 1,
      ширинаДок: document.documentElement.scrollWidth,
      шире,
      навВидно: nav ? getComputedStyle(nav).display !== 'none' && nav.getBoundingClientRect().height > 0 : null,
      навВысота: nav ? Math.round(nav.getBoundingClientRect().height) : null,
      ссылокВНаве: nav ? nav.querySelectorAll('button, a').length : 0,
      мелкихКнопок: кнопки,
      высотаСтр: document.documentElement.scrollHeight,
      бургер: !!document.querySelector('.burger') && getComputedStyle(document.querySelector('.burger')).display !== 'none',
      подМеню: Math.round(document.querySelector('.main').getBoundingClientRect().top),
      навВидимых: nav ? [...nav.querySelectorAll('.ent.top')].filter(e => {
        const r = e.getBoundingClientRect();
        return r.left >= -1 && r.right <= win + 1 && r.width > 0;
      }).length : 0,
      навВсего: nav ? nav.querySelectorAll('.ent.top').length : 0,
      срезано: [...document.querySelectorAll('.u-t, .wk-t, .co-tbl td[data-k="title"], .rk span, .ls .t')]
        .filter(e => e.scrollWidth > e.clientWidth + 2 && e.clientWidth < 90)
        .map(e => `${e.className || e.tagName}="${e.textContent.slice(0,14)}" ${Math.round(e.clientWidth)}px`)
        .slice(0, 4),
      строкНава: nav ? new Set([...nav.querySelectorAll('.ent.top')]
        .map(e => Math.round(e.getBoundingClientRect().top))).size : 0,
      картаНаСвоейСтроке: (() => {
        const к = document.querySelector('.picker'), п = document.querySelector('.ent.top');
        if (!к || !п) return null;
        return Math.round(к.getBoundingClientRect().top) !== Math.round(п.getBoundingClientRect().top);
      })(),
      навЛистается: nav ? nav.scrollWidth > nav.clientWidth + 1 : false,
    };
  });
  console.log(`${s.padEnd(8)} вбок:${d.едетВбок ? 'ДА ' + d.ширинаДок : 'нет'} · бургер:${d.бургер} · содержимое с ${d.подМеню}px · мелких:${d.мелкихКнопок} ${d.срезано.length ? '· СРЕЗАНО: ' + d.срезано.join(', ') : ''} · ошибки:${errs.length}`);
  if (d.шире.length) console.log(`         шире экрана: ${d.шире.join(', ')}`);
  await p.screenshot({ path: `${process.env.SHOT}-${s}.png` });
}
await ctx.close(); await b.close();

import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const стр = ['pult','where','tasks','unknown','read','corpus','depends'];
const ctx = await b.newContext({ viewport: { width: 1440, height: 1000 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 80)));
// Кириллица, пришедшая от СЕРВЕРА, — слова проекта, они не переводятся.
// Ищем её только в том, что нарисовал клиент: в шапках и подписях разделов.
let всегоРу = 0;
for (const s of стр) {
  await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=${s}&lang=en`, { waitUntil: 'domcontentloaded' });
  await p.waitForTimeout(2200);
  const d = await p.evaluate(() => {
    const бери = (sel) => [...document.querySelectorAll(sel)].map(e => e.textContent.trim()).filter(Boolean);
    const хром = [...бери('.side .ent-n'), ...бери('h1'), ...бери('.head .prov'),
                  ...бери('.empty'), ...бери('button.ghost'), ...бери('input[placeholder]').concat(
                  [...document.querySelectorAll('input[placeholder]')].map(e => e.placeholder))];
    return { ру: хром.filter(t => /[а-яА-ЯёЁ]/.test(t)), всего: хром.length };
  });
  всегоРу += d.ру.length;
  console.log(`  ${s.padEnd(8)} подписей ${String(d.всего).padStart(3)} · по-русски ${d.ру.length}${d.ру.length ? ' → ' + d.ру.slice(0,3).join(' | ').slice(0,80) : ''}`);
}
say('шапки и подписи по-английски', всегоРу === 0, `${всегоРу} русских`);
// И обратно: по-русски всё на месте.
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=pult&lang=ru`, { waitUntil: 'domcontentloaded' });
await p.waitForTimeout(1800);
const ру = await p.locator('.side .ent-n').allInnerTexts();
say('по-русски разделы на месте', ру.every(t => /[а-яА-ЯёЁ]/.test(t)), ру.join(' · '));
say('без ошибок', errs.length === 0, errs[0] ?? '');
await b.close();

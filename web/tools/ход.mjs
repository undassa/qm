import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(15000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));

console.log('══ Корпус: запись-текст показывается и ссылки в ней ходят');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-kinds button', { state: 'attached' });
await p.locator('.co-kinds button', { hasText: 'решение' }).first().click();
await p.waitForTimeout(1600);
await p.locator('.co-tbl tbody tr').nth(1).click();
await p.waitForSelector('.co-one'); await p.waitForTimeout(1200);
const текст = (await p.locator('.co-one').innerText()).replace(/\s+/g, ' ');
say('тело записи показано', текст.length > 400, `${текст.length} знаков`);
const ссылок = await p.locator('.co-one a.go').count();
say('в теле есть ссылки', ссылок > 0, `${ссылок} штук`);
if (ссылок) {
  const a = p.locator('.co-one a.go').first();
  const вид = await a.getAttribute('data-kind'), имя = await a.getAttribute('data-id');
  await a.click(); await p.waitForTimeout(2200);
  say('щелчок увёл в документы', p.url().includes('page=read'));
  say('проект не потерян', p.url().includes('project='));
  const шапка = (await p.locator('.doc-head').innerText().catch(() => '')).replace(/\s+/g, ' ');
  say('открылось просимое', шапка.includes(имя || вид), `ждали ${вид}:${имя || '(одиночка)'} · видим ${шапка.slice(0, 40)}`);
  await p.goBack(); await p.waitForTimeout(1600);
  say('назад возвращает в корпус', p.url().includes('page=corpus'));
}

console.log('══ Документы: ссылки на одиночек стали ссылками');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-0156`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.reader-doc'); await p.waitForTimeout(1500);
const кн = await p.locator('.doc-secs button').all();
for (const к of кн.slice(0, 6)) await к.click().catch(() => {});
await p.waitForTimeout(1800);
const всего = await p.locator('a.go').count(), одиноч = await p.locator('a.go[data-id=""]').count();
say('ссылки есть, включая одиночек', всего > 0 && одиноч > 0, `${всего} всего, ${одиноч} к одиночкам`);

console.log('══ Адрес со ссылкой открывается снаружи');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=glossary`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.doc-head'); await p.waitForTimeout(1500);
const г = (await p.locator('.doc-head').innerText()).replace(/\s+/g, ' ');
say('одиночка открылась по адресу', г.includes('glossary'), г.slice(0, 50));
say('без ошибок', errs.length === 0, errs[0] ?? '');
await b.close();

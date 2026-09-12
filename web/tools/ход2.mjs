import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));

console.log('══ Корпус: ADR-0134 — тридцать одна ссылка в наборе');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-kinds button', { state: 'attached' });
await p.locator('.co-kinds button', { hasText: 'решение' }).first().click();
await p.waitForTimeout(1600);
await p.locator('.co-q').fill('ADR-0134'); await p.waitForTimeout(900);
await p.locator('.co-tbl tbody tr', { hasText: 'ADR-0134' }).first().click();
await p.waitForSelector('.co-one'); await p.waitForTimeout(1500);
const текст = (await p.locator('.co-one').innerText()).replace(/\s+/g, ' ');
const ссылок = await p.locator('.co-one a.go').count();
const одиноч = await p.locator('.co-one a.go[data-id=""]').count();
say('тело показано', текст.length > 3000, `${текст.length} знаков`);
say('ссылки нарисованы', ссылок > 10, `${ссылок} штук, из них к одиночкам ${одиноч}`);
say('одиночки тоже ссылки', одиноч > 0);
say('скобок «(decision:...)» в тексте не осталось', !/\(decision:|\(question:|\(glossary:/.test(текст));
if (ссылок) {
  const a = p.locator('.co-one a.go').first();
  const вид = await a.getAttribute('data-kind'), имя = await a.getAttribute('data-id');
  await a.click(); await p.waitForTimeout(2400);
  say('щелчок увёл в документы', p.url().includes('page=read'), p.url().replace('http://127.0.0.1:8096',''));
  say('проект не потерян', p.url().includes('project='));
  const шапка = (await p.locator('.doc-head').innerText().catch(() => '')).replace(/\s+/g, ' ');
  say('открылось просимое', шапка.includes(имя || вид), `ждали ${вид}:${имя || '(одиночка)'} · видим ${шапка.slice(0, 44)}`);
  await p.goBack(); await p.waitForTimeout(1800);
  say('назад возвращает в корпус', p.url().includes('page=corpus'));
}
console.log('══ Одиночка по ссылке из документа');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=read&kind=decision&id=ADR-0138`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.doc-secs button');
let ссылок = 0;
for (let попытка = 0; попытка < 4 && !ссылок; попытка++) {
  await p.locator('.doc-sum button.ghost').click().catch(() => {});
  for (let ждём = 0; ждём < 20 && !ссылок; ждём++) {
    await p.waitForTimeout(700);
    ссылок = await p.locator('a.go').count();
  }
}
console.log(`     (раскрыто разделов: ${await p.locator('.sec-h.on').count()})`);
const вд = ссылок;
say('в читалке ссылки на месте', вд > 0, `${вд} штук`);
if (вд) {
  const один = p.locator('a.go[data-id=""]').first();
  if (await один.count()) {
    const вид = await один.getAttribute('data-kind');
    await один.click(); await p.waitForTimeout(2200);
    say('щелчок по одиночке открыл её', (await p.locator('.doc-head').innerText()).includes(вид || ''), вид ?? '');
  } else say('одиночка среди раскрытых', false, 'не попалась — раскрыто мало разделов');
}
say('без ошибок', errs.length === 0, errs[0] ?? '');
await b.close();

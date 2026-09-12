import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 1000 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));

async function запись(род, имя) {
  await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus`, { waitUntil: 'domcontentloaded' });
  await p.waitForSelector('.co-kinds li button');
  await p.locator('.co-kinds li button', { hasText: род }).first().click();
  await p.waitForTimeout(1500);
  await p.locator('.co-q').fill(имя); await p.waitForTimeout(900);
  await p.locator('.co-tbl tbody tr', { hasText: имя }).first().click();
  await p.waitForSelector('.co-imp'); await p.waitForTimeout(1500);
}

console.log('══ требование FR-CFG-01 — замер двери: переоткроет 3, затронет 5, глубина 3');
await запись('требование', 'FR-CFG-01');
const счёт = (await p.locator('.co-imp-n').innerText()).replace(/\s+/g, ' ');
say('счёт последствий показан', /3/.test(счёт) && /5/.test(счёт), счёт);
const шт = await p.locator('.co-imp-steps .co-imp-i').count();
const пере = await p.locator('.co-imp-i.reopens').count();
say('перечислено то, что стоит на записи', шт === 5, `${шт} записей`);
say('переоткрываемые помечены', пере === 3, `${пере} со знаком ↻`);
const наб = await p.locator('.co-imp-sets li').count();
say('наборы связей показаны', наб === 4, `${наб} наборов`);
const два = await p.locator('.co-imp-h').all();
const низ1 = (await два[0].boundingBox()).y + (await два[0].boundingBox()).height;
const верх2 = (await два[1].boundingBox()).y;
say('второй заголовок не налезает', верх2 - низ1 > 40, `зазор ${Math.round(верх2 - низ1)}px`);
say('заголовки названы по-разному', (await два[1].innerText()).toLowerCase().includes('объявленные'), await два[1].innerText());
say('пояснение двери на месте', (await p.locator('.co-imp-say').first().innerText()).includes('СТОИТ НА'));

const a = p.locator('.co-imp-steps .co-imp-i').first();
const вид = await a.getAttribute('data-kind'), ид = await a.getAttribute('data-id');
await a.click(); await p.waitForTimeout(2200);
say('щелчок по последствию уводит к нему', p.url().includes(`kind=${вид}`) && p.url().includes(`id=${ид}`), `${вид}:${ид}`);
say('проект не потерян', p.url().includes('project='));

console.log('══ род, у которого наборы связей не объявлены');
await запись('вопрос', 'Q-325');
const т = (await p.locator('.co-imp').innerText()).replace(/\s+/g, ' ');
say('сказано «не объявлены», а не показан пустой список', т.includes('не объявлены'), т.slice(т.indexOf('Связи'), т.indexOf('Связи') + 78));

console.log('══ запись, на которой не стоит ничего');
await запись('решение', 'ADR-0134');
const т2 = (await p.locator('.co-imp').innerText()).replace(/\s+/g, ' ');
say('сказано, что править безопасно', т2.includes('безопасно'), т2.slice(0, 90));
// Ограда кода: набор пишет код ими, и разборщик их не знал — комментарии
// шелла выходили заголовками.
const оград = await p.locator('.co-one pre').count();
const решёток = (await p.locator('.co-one h3, .co-one h4, .co-one h5').allInnerTexts()).filter(t => /^\s*(awk|grep|#!|SELECT)/i.test(t)).length;
say('код показан оградой, а не заголовками', оград > 0 && решёток === 0, `${оград} блоков кода, ${решёток} ложных заголовков`);
say('голых ``` в тексте не осталось', !(await p.locator('.co-one').innerText()).includes('```'));
say('без ошибок', errs.length === 0, errs[0] ?? '');
await p.screenshot({ path: `${process.env.SHOT}-каскад.png` });
await b.close();

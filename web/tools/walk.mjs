import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 950 } });
await ctx.addCookies([{ name: 'mh_identity', value: process.env.MH_TOKEN, url: 'http://127.0.0.1:8096' }]);
const p = await ctx.newPage();
const errs = [];
p.on('pageerror', e => errs.push('PAGEERROR ' + String(e).slice(0, 110)));
p.on('console', m => { if (m.type() === 'error') errs.push(m.text().slice(0, 110)); });
const say = (what, ok, note = '') => console.log(`${ok ? '  ✓' : '  ✗'} ${what}${note ? ' — ' + note : ''}`);

await p.goto('http://127.0.0.1:8096/next/?page=where', { waitUntil: 'networkidle' });
await p.waitForTimeout(1100);

// 1. щёлкнуть по каждой фазе
let phases = 0;
for (const seg of await p.$$('.rail-seg')) { await seg.click(); await p.waitForTimeout(220); phases++; }
say(`фазы переключаются (${phases})`, phases === 6);

// 2. развернуть пройденные
const fold = await p.$('.fold');
if (fold) { await fold.click(); await p.waitForTimeout(300); }
say('пройденные разворачиваются', !!fold);

// 3. раскрыть пункт и свернуть
const it = await p.$('.item-head');
await it.click(); await p.waitForTimeout(300);
const opened = !!(await p.$('.item.open'));
await it.click(); await p.waitForTimeout(200);
say('пункт раскрывается и закрывается', opened && !(await p.$('.item.open')));

// 4. палитра клавишами
await p.keyboard.press('Control+k'); await p.waitForTimeout(300);
await p.keyboard.type('ADR'); await p.waitForTimeout(900);
await p.keyboard.press('ArrowDown'); await p.waitForTimeout(120);
const on = await p.$$eval('.pal-row.on', e => e.length);
await p.keyboard.press('Escape'); await p.waitForTimeout(250);
say('палитра: стрелки и Esc', on === 1 && !(await p.$('.pal')));

// 5. каждый раздел из боковика
let pages = 0, bad = [];
for (const btn of await p.$$('.side .ent.top')) {
  const name = (await btn.innerText()).split('\n')[0];
  errs.length = 0;
  await btn.click(); await p.waitForTimeout(900);
  pages++;
  if (errs.length) bad.push(`${name}: ${errs[0]}`);
}
say(`разделы открываются (${pages})`, bad.length === 0, bad.join(' ; '));

// 6. чтение: вид → документ → ссылка → назад
await p.goto('http://127.0.0.1:8096/next/?page=read', { waitUntil: 'networkidle' });
await p.waitForTimeout(900);
await (await p.$('.reader-kinds :text-is("task")')).click(); await p.waitForTimeout(1000);
const auto = await p.$eval('.reader-doc', e => e.innerText.slice(0, 12)).catch(() => '');
say('документ открылся сам', auto.includes('task'));

await b.close();

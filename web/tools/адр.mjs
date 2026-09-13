import { chromium } from 'playwright';
const b = await chromium.launch();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
const P = '308ed7a2-d18f-4a76-a9c4-c792ce7de0d3';
const ctx = await b.newContext({ viewport: { width: 1440, height: 1000 } });
ctx.setDefaultTimeout(20000);
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_EDGE_SECRET, 'x-mh-principal': 'undassa' });
const p = await ctx.newPage();
const errs = []; p.on('pageerror', e => errs.push(String(e).slice(0, 90)));

console.log('══ Адрес записи открывается напрямую');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus&kind=requirement&id=FR-CFG-01`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-one'); await p.waitForTimeout(1500);
say('открылась именно она', (await p.locator('.co-one-id').innerText()).trim() === 'FR-CFG-01', await p.locator('.co-one-id').innerText());
say('род выбран под неё', (await p.locator('.co-kinds li button.on').innerText()).includes('требование'), (await p.locator('.co-kinds li button.on').innerText()).replace(/\s+/g,' '));
await p.waitForTimeout(1200);
say('имя осталось в адресе после загрузки', p.url().includes('id=FR-CFG-01'), p.url().split('?')[1]);

console.log('══ Выбор записи попадает в адрес');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-tbl tbody tr'); await p.waitForTimeout(1400);
await p.locator('.co-q').fill('FR-CFG-03'); await p.waitForTimeout(900);
await p.locator('.co-tbl tbody tr', { hasText: 'FR-CFG-03' }).first().click();
await p.waitForSelector('.co-one'); await p.waitForTimeout(1200);
say('имя записи в адресе', p.url().includes('id=FR-CFG-03'), p.url().split('?')[1]);
say('род в адресе', p.url().includes('kind=requirement'));

console.log('══ Смена рода правит адрес, запись уходит');
await p.locator('.co-back').click(); await p.waitForTimeout(700);
await p.locator('.co-kinds li button', { hasText: 'риск' }).first().click(); await p.waitForTimeout(1500);
say('род сменился в адресе', p.url().includes('kind=risk'), p.url().split('?')[1]);
say('имя прежней записи убрано', !p.url().includes('id='));

console.log('══ Ссылка из записи уводит в документы и не путает Корпус');
await p.goto(`http://127.0.0.1:8096/next/?project=${P}&page=corpus&kind=requirement&id=FR-CFG-01`, { waitUntil: 'domcontentloaded' });
await p.waitForSelector('.co-imp'); await p.waitForTimeout(1500);
await p.locator('.co-imp-steps .co-imp-i').first().click(); await p.waitForTimeout(2200);
say('ушли в документы', p.url().includes('page=read'), p.url().split('?')[1]);
await p.goBack(); await p.waitForTimeout(2000);
say('назад вернул в корпус', p.url().includes('page=corpus'));
say('и вернул ту же запись', (await p.locator('.co-one-id').innerText().catch(()=>'')).trim() === 'FR-CFG-01', await p.locator('.co-one-id').innerText().catch(()=>'нет'));
say('без ошибок', errs.length === 0, errs[0] ?? '');
await b.close();

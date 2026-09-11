import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 1150 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
const p = await ctx.newPage();
const errs = [];
p.on('pageerror', e => errs.push('PAGEERROR ' + String(e).slice(0, 120)));
p.on('console', m => { if (m.type() === 'error') errs.push(m.text().slice(0, 120)); });
await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&page=depends', { waitUntil: 'networkidle' });
await p.waitForTimeout(1500);
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
say('строк карты', (await p.locator('.dep-row').count()) > 0, String(await p.locator('.dep-row').count()));
say('родов в списке', (await p.locator('.dep-list li').count()) > 0, String(await p.locator('.dep-list li').count()));
const node = p.locator('.dep-node').filter({ hasText: 'требование' }).first();
if (await node.count()) {
  await node.click(); await p.waitForTimeout(900);
  say('род выбирается', (await p.locator('.dep-pick').count()) > 0);
  const opts = await p.locator('.dep-pick option').count();
  say('имена подгрузились', opts > 1, `${opts} вариантов`);
  if (opts > 1) {
    await p.selectOption('.dep-pick', { index: 1 }); await p.waitForTimeout(1200);
    const hits = await p.locator('.dep-hits li').count();
    say('последствия показаны', hits > 0, `${hits} строк`);
    say('сводка есть', (await p.locator('.dep-sum').count()) > 0, (await p.locator('.dep-sum').first().innerText().catch(()=>'')).replace(/\s+/g,' ').slice(0,60));
  }
} else say('род найден', false);
say('без ошибок в консоли', errs.length === 0, errs.slice(0,2).join(' | '));
await p.screenshot({ path: process.env.SHOT, fullPage: false });
await b.close();

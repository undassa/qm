import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1100, height: 900 } });
await ctx.setExtraHTTPHeaders({ 'x-mh-edge': process.env.MH_TOKEN, 'x-mh-principal': process.env.MH_PRINCIPAL });
const p = await ctx.newPage();
const say = (w, ok, n='') => console.log(`  ${ok ? '✓' : '✗'} ${w}${n ? ' — ' + n : ''}`);
await p.goto('http://127.0.0.1:8096/next/?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3', { waitUntil: 'networkidle' });
await p.waitForTimeout(1500);

// Форма раскрывается в строке, а не уводит на другой экран.
await p.locator('.pu-asks .pu-do').first().click();
await p.waitForTimeout(400);
say('форма раскрылась', (await p.locator('.pu-draft').count()) === 1);
say('кнопка заперта без текста', await p.locator('.pu-form .pu-do').isDisabled());
await p.locator('.pu-draft').fill('проба');
await p.waitForTimeout(200);
say('кнопка отперлась с текстом', !(await p.locator('.pu-form .pu-do').isDisabled()));

// Путь записи целиком — на двери, которая ОТКАЗЫВАЕТ: набор не трогаем.
const r = await p.evaluate(async () => {
  const res = await fetch('/api/projects/308ed7a2-d18f-4a76-a9c4-c792ce7de0d3/tool/kind-proves', {
    method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ kind: 'story', proves: true, why: 'проба пути записи' }),
  });
  return { ok: res.ok, body: await res.json() };
});
say('POST дошёл до двери', r.ok, JSON.stringify(r.body).slice(0, 60));
say('дверь отказала по существу', r.body?.status === 'proves_nothing', r.body?.status ?? '');
say('отказ несёт причину', typeof r.body?.why === 'string' && r.body.why.length > 20);
await b.close();

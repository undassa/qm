import { chromium } from 'playwright';
const b = await chromium.launch();
const ctx = await b.newContext({ viewport: { width: 1440, height: 420 } });
await ctx.addCookies([{ name: 'mh_identity', value: process.env.MH_TOKEN, url: 'http://127.0.0.1:8096' }]);
const p = await ctx.newPage();
for (const [n, path] of JSON.parse(process.argv[3])) {
  await p.goto('http://127.0.0.1:8096/next/' + path.replace('?', '?project=308ed7a2-d18f-4a76-a9c4-c792ce7de0d3&'), { waitUntil: 'networkidle' });
  await p.waitForTimeout(1000);
  await p.screenshot({ path: process.argv[2] + '/' + n + '.png' });
}
await b.close();

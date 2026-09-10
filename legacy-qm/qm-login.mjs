// Вход в qm двумя слоями:
//   1) подпись источника  (CORE_SIGNING_SECRET)   — «этой службе доверяют»
//   2) портальный токен   (PORTAL_IDENTITY_SECRET) — «и вот кто я»
import { createHmac, createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { CompactSign } from "jose";
const env = Object.fromEntries(readFileSync(process.argv[2], "utf8").split("\n")
  .filter((l) => l && !l.startsWith("#")).map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]));
const [, , , who = "undassa", method = "GET", path = "/v1/admin/whoami", body = ""] = process.argv;
const enc = new TextEncoder();
const kid = createHash("sha256").update(env.PORTAL_IDENTITY_SECRET).digest("base64url").slice(0, 8);
const portal = await new CompactSign(enc.encode(JSON.stringify({ p: who, exp: Math.floor(Date.now()/1000)+3600 })))
  .setProtectedHeader({ alg: "HS256", kid }).sign(enc.encode(env.PORTAL_IDENTITY_SECRET));
const ts = Math.floor(Date.now()/1000);
const sig = `v0=${createHmac("sha256", env.CORE_SIGNING_SECRET).update(`v0:${ts}:${method}\n${path}\n${body}`).digest("hex")}`;
const r = await fetch(`http://127.0.0.1:${env.PORT}${path}`, { method,
  headers: { "x-timestamp": String(ts), "x-signature": sig, "x-portal-identity": portal, "x-admin-actor": `${who}@${env.ORG_ID}`, "content-type": "application/json" },
  body: body || undefined });
console.log(`${r.status} ${r.statusText}  ${(await r.text()).slice(0, 400)}`);

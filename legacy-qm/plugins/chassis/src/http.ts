import type { IncomingMessage, ServerResponse } from "node:http";

export class PayloadTooLargeError extends Error {
  constructor() {
    super("request body too large");
    this.name = "PayloadTooLargeError";
  }
}

export function json(res: ServerResponse, status: number, body: unknown): void {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
}

export async function readBody(req: IncomingMessage, maxBytes = Infinity): Promise<string> {
  const chunks: Buffer[] = [];
  let size = 0;
  for await (const c of req) {
    size += (c as Buffer).length;
    if (size > maxBytes) throw new PayloadTooLargeError();
    chunks.push(c as Buffer);
  }
  return Buffer.concat(chunks).toString("utf8");
}

export function cookie(req: IncomingMessage, name: string): string | null {
  const m = (req.headers.cookie ?? "").match(new RegExp(`(?:^|;\\s*)${name}=([^;]+)`));
  return m ? decodeURIComponent(m[1] ?? "") || null : null;
}

export function escapeHtml(s: string): string {
  return s.replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c] ?? c,
  );
}

const MARK_SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">' +
  "<style>path{stroke:#171a1c}@media(prefers-color-scheme:dark){path{stroke:#e8edf0}}</style>" +
  '<g fill="none" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round">' +
  '<path d="M8 27V6"/><path d="M8 17a4 4 0 0 1 8 0v10"/><path d="M16 17a4 4 0 0 1 8 0v10"/>' +
  "</g></svg>";

export function serveFavicon(res: ServerResponse, emoji: string | undefined, cacheControl: string): void {
  res.writeHead(200, { "content-type": "image/svg+xml; charset=utf-8", "cache-control": cacheControl });
  res.end(
    emoji
      ? `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><text y=".9em" font-size="90" text-anchor="middle" x="50">${emoji}</text></svg>`
      : MARK_SVG,
  );
}

import { Отказ } from "./api";
import { type Lang, say } from "./say";

/**
 * Отказ словами. Дверь объясняет сама — её слова и показываются; наши только
 * там, где она промолчала. `String(e)` вместо этого печатал «Error: 502 /api/…».
 */
export function отказом(l: Lang, e: unknown): string {
  if (e instanceof Отказ) {
    if (e.message) return e.message;
    return e.status === 0 ? say(l, "er.noNet") : `${say(l, "er.silent")} (${e.status})`;
  }
  const t = e instanceof Error ? e.message : String(e);
  return t.replace(/^Error:\s*/, "") || say(l, "er.silent");
}

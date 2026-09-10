import { loadConfig } from "../config.ts";
import { buildApp, stopWithBackstop } from "../wiring.ts";

const config = loadConfig();
const built = buildApp(config);
await built.config.hydrate?.();
await built.identity.hydrate();
const { runtime } = built;
runtime.start();
console.log(`[mh:worker] draining runs (org=${config.orgId}, runStore=${config.runStore}, workers=${config.workers})`);

let shuttingDown = false;
function shutdown(signal: string): void {
  if (shuttingDown) return;
  shuttingDown = true;
  console.log(`[mh:worker] ${signal} received, stopping`);
  stopWithBackstop(runtime, config.shutdownDrainMs, "mh:worker");
}
process.on("SIGINT", () => shutdown("SIGINT"));
process.on("SIGTERM", () => shutdown("SIGTERM"));

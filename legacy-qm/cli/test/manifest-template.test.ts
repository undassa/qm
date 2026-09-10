import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { renderSlackManifests } from "../src/slack-manifests.ts";
import type { MhConfig } from "../src/config.ts";

interface SlackManifest {
  oauth_config: { scopes: { bot: string[] } };
  settings: { event_subscriptions: { bot_events: string[] } };
}

const manifest = (url: URL): SlackManifest => JSON.parse(readFileSync(url, "utf8")) as SlackManifest;

const manifestConfig = (botName?: string): MhConfig =>
  ({
    orgId: "acme",
    publicUrl: "https://mh.example.com",
    ...(botName ? { botName } : {}),
  }) as MhConfig;

test("rendered Slack manifests default to mh naming", () => {
  const { bot, sso } = renderSlackManifests(manifestConfig());
  assert.match(bot, /name: mh\n/);
  assert.match(bot, /display_name: mh\n/);
  assert.match(bot, /description: mh workspace agent for acme\n/);
  assert.match(sso, /name: mh SSO\n/);
  assert.match(sso, /description: Sign in to your mh deployment with Slack\n/);
});

test("botName renames both rendered Slack manifests", () => {
  const { bot, sso } = renderSlackManifests(manifestConfig("straylight"));
  assert.match(bot, /name: straylight\n/);
  assert.match(bot, /display_name: straylight\n/);
  assert.match(bot, /description: straylight workspace agent for acme\n/);
  assert.match(sso, /name: straylight SSO\n/);
  assert.match(sso, /description: Sign in to your straylight deployment with Slack\n/);
  assert.doesNotMatch(bot, /\bmh\b/);
});

test("the CLI Slack manifest template matches the plugin's scopes and events", () => {
  const template = manifest(new URL("../templates/slack-manifest.json", import.meta.url));
  const plugin = manifest(new URL("../../src/slack/manifest.json", import.meta.url));
  assert.deepEqual([...template.oauth_config.scopes.bot].sort(), [...plugin.oauth_config.scopes.bot].sort());
  assert.deepEqual(
    [...template.settings.event_subscriptions.bot_events].sort(),
    [...plugin.settings.event_subscriptions.bot_events].sort(),
  );
});

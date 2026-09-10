import test from "node:test";
import assert from "node:assert/strict";
import { projectFeatures } from "../src/projects/feature-projection.ts";

const SOURCE = {
  paths: [
    "10-intent/functional/monitors.md",
    "10-intent/functional/README.md",
    "10-intent/functional/onboarding.md",
    "10-intent/srs.md",
  ],
  titleOf: (p: string) => (p.includes("monitors") ? "Мониторы и подавление" : `Заголовок ${p}`),
  sectionTitlesOf: (p: string) => {
    if (p.endsWith("monitors.md")) {
      return [
        "Мониторы и подавление",
        "Почему эта фича появилась последней",
        "US-MON-01 · Завести проверку",
        "US-MON-02 · Всплеск не будит",
        "US-MON-01 · повтор в другом разделе",
      ];
    }
    return p.endsWith("onboarding.md") ? ["Первый день", "US-ONB-01 · Прийти и увидеть"] : [];
  },
};

test("область берёт свои истории из заголовков разделов, README областью не считается", () => {
  const { features, links } = projectFeatures(SOURCE);
  assert.deepEqual(
    features.map((f) => f.id),
    ["monitors", "onboarding"],
  );
  assert.deepEqual(
    links.map((l) => l.storyId),
    ["US-MON-01", "US-MON-02", "US-ONB-01"],
  );
});

test("одна история считается областью один раз, даже если названа дважды", () => {
  const { features } = projectFeatures(SOURCE);
  assert.equal(features.find((f) => f.id === "monitors")!.stories, 2, "повтор не раздувает счёт");
});

test("раздел без идентификатора истории связью не становится", () => {
  const { links } = projectFeatures(SOURCE);
  assert.ok(
    !links.some((l) => l.storyId.includes("Почему")),
    "«Почему эта фича появилась последней» — это проза, а не история",
  );
});

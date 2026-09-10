import assert from "node:assert/strict";
import test from "node:test";
import { chainOf, layoutWaves, type WaveTask } from "../src/task-waves.ts";

const task = (id: string, dependsOn: string[] = [], state = "not_started"): WaveTask => ({
  id,
  dependsOn,
  state,
  blockedBy: dependsOn.length,
});

test("волна — это глубина в графе, а не порядок объявления", () => {
  const layout = layoutWaves([task("C", ["B"]), task("A"), task("B", ["A"]), task("D", ["A"])]);
  assert.deepEqual(
    layout.waves.map((w) => w.map((t) => t.id).sort()),
    [["A"], ["B", "D"], ["C"]],
    "B и D не зависят друг от друга — значит одна волна и два агента разом",
  );
  assert.equal(layout.chainLength, 3, "критическая цепочка — число волн");
  assert.equal(layout.peak, 2, "самая широкая волна задаёт предел параллелизма");
});

test("задача уходит на волну после самого позднего предшественника, а не самого раннего", () => {
  const layout = layoutWaves([task("A"), task("B", ["A"]), task("C", ["A", "B"])]);
  assert.equal(layout.depthOf.get("C"), 2, "C ждёт B, поэтому не может встать рядом с ним");
});

test("зависимость вне набора не углубляет волну", () => {
  const layout = layoutWaves([task("A", ["ИЗВНЕ"]), task("B", ["A"])]);
  assert.equal(layout.depthOf.get("A"), 0, "предшественника нет в наборе — считать нечего");
  assert.equal(layout.chainLength, 2);
});

test("цикл не вешает расчёт", () => {
  const layout = layoutWaves([task("A", ["B"]), task("B", ["A"])]);
  assert.equal(layout.waves.length > 0, true, "расчёт завершается, а не уходит в рекурсию");
});

test("готовы к работе — незакрытые без незакрытых предшественников", () => {
  const layout = layoutWaves([
    { id: "A", dependsOn: [], state: "closed", blockedBy: 0 },
    { id: "B", dependsOn: ["A"], state: "not_started", blockedBy: 0 },
    { id: "C", dependsOn: ["B"], state: "not_started", blockedBy: 1 },
  ]);
  assert.equal(layout.ready, 1, "только B — C ещё ждёт");
});

test("цепочка задачи берёт и предков, и потомков, но не соседей", () => {
  const tasks = [task("A"), task("B", ["A"]), task("C", ["B"]), task("X"), task("Y", ["X"])];
  assert.deepEqual([...chainOf(tasks, "B")].sort(), ["A", "B", "C"], "ветка X→Y к B отношения не имеет");
});

const dev = (id: string, dependsOn: string[] = [], state = "not_started"): WaveTask => ({
  id,
  dependsOn,
  state,
  blockedBy: dependsOn.length,
  kind: "dev",
});
const test_ = (id: string, dependsOn: string[] = [], state = "not_started"): WaveTask => ({
  id,
  dependsOn,
  state,
  blockedBy: dependsOn.length,
  kind: "test",
});

test("ADR-0082: весь трек проверок идёт до всего кода, а не пара к паре", () => {
  // у M1-T1 нет зависимостей вовсе, но он всё равно уходит за трек проверок
  const layout = layoutWaves([dev("M1-T1"), test_("V1-T1"), test_("V1-T2", ["V1-T1"])]);
  assert.equal(layout.testWaves, 2, "проверки заняли две волны");
  assert.deepEqual(
    layout.waves.map((w) => w.map((t) => t.id)),
    [["V1-T1"], ["V1-T2"], ["M1-T1"]],
    "код начинается после последней проверки, хотя его собственные зависимости пусты",
  );
});

test("пока открыта хоть одна проверка, код не считается готовым", () => {
  const open = layoutWaves([dev("M1-T1"), test_("V1-T1")]);
  assert.equal(open.ready, 1, "готова только проверка");

  const closed = layoutWaves([dev("M1-T1"), test_("V1-T1", [], "closed")]);
  assert.equal(closed.ready, 1, "трек закрыт — код освободился");
  assert.equal(
    closed.waves
      .at(-1)!
      .map((t) => t.id)
      .join(),
    "M1-T1",
  );
});

test("без трека проверок правило не мешает: код готов сразу", () => {
  const layout = layoutWaves([dev("M1-T1"), dev("M1-T2", ["M1-T1"])]);
  assert.equal(layout.testWaves, 0);
  assert.equal(layout.ready, 1, "проверок нет — ждать нечего");
});

const inM = (id: string, milestoneId: string, artifacts: string[] = [], dependsOn: string[] = []): WaveTask => ({
  id,
  dependsOn,
  state: "not_started",
  blockedBy: dependsOn.length,
  kind: "dev",
  milestoneId,
  artifacts,
});

test("этапы идут по номеру: задача M2 не встаёт рядом с задачей M0", () => {
  const layout = layoutWaves([inM("M0-T1", "M0"), inM("M2-T1", "M2"), inM("M8-T1", "M8")]);
  assert.deepEqual(
    layout.waves.map((w) => w.map((t) => t.id)),
    [["M0-T1"], ["M2-T1"], ["M8-T1"]],
    "«частично» не бывает: этап проходится целиком, потом следующий",
  );
});

test("общий артефакт разводит задачи по разным волнам, даже без связи в плане", () => {
  const together = layoutWaves([inM("M1-T1", "M1"), inM("M1-T2", "M1")]);
  assert.equal(together.waves.length, 1, "разные экраны — можно разом");

  const clash = layoutWaves([inM("M1-T1", "M1", ["Экраны:SCR-OPS-11"]), inM("M1-T2", "M1", ["Экраны:SCR-OPS-11"])]);
  assert.equal(clash.waves.length, 2, "один экран на двоих — только по очереди");
  assert.equal(clash.idealChain, 1, "без учёта артефактов хватило бы одной волны");
});

test("цена общих артефактов видна как разница между цепочкой и идеалом", () => {
  const layout = layoutWaves([
    inM("M1-T1", "M1", ["Операции контракта:POST /a"]),
    inM("M1-T2", "M1", ["Операции контракта:POST /a"]),
    inM("M1-T3", "M1", ["Операции контракта:POST /a"]),
  ]);
  assert.equal(layout.idealChain, 1);
  assert.equal(layout.chainLength, 3, "три задачи за одну операцию — три волны");
});

test("без порядка этапов задачи разных вех сходятся в одной волне", () => {
  // Ровно то, что показывает доска «Волны myack»: в первой волне сходятся M0,
  // M2 и M8, потому что ни одна из них ничего не ждёт.
  const layout = layoutWaves([inM("M0-T1", "M0"), inM("M2-T1", "M2"), inM("M8-T1", "M8")], {
    milestoneFloors: false,
  });
  assert.deepEqual(
    layout.waves.map((w) => w.map((t) => t.id).sort()),
    [["M0-T1", "M2-T1", "M8-T1"]],
  );
});

test("общие артефакты разводят задачи и без порядка этапов", () => {
  const layout = layoutWaves(
    [inM("M0-T1", "M0", ["Экраны:SCR-OPS-11"]), inM("M8-T1", "M8", ["Экраны:SCR-OPS-11"])],
    { milestoneFloors: false },
  );
  assert.equal(layout.waves.length, 2, "один экран на двоих — по очереди, чей бы этап ни был");
  assert.equal(layout.idealChain, 1);
});

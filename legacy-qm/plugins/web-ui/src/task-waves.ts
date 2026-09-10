export interface WaveTask {
  id: string;
  dependsOn: string[];
  state: string;
  blockedBy: number;
  /** "test" — трек проверок, "dev" — код. */
  kind?: string;
  /** Веха задачи: этапы идут по номеру, «частично» не бывает. */
  milestoneId?: string;
  /** Экраны и операции контракта, которых задача касается. */
  artifacts?: string[];
}

/**
 * Волна — глубина задачи в графе зависимостей: сколько шагов цепочки лежит перед ней.
 * Задачи одной волны не зависят друг от друга, поэтому их можно вести разными агентами разом.
 * Число волн — критическая цепочка: короче неё план не пройти, сколько агентов ни дай.
 */
export interface WaveLayout<T extends WaveTask> {
  waves: T[][];
  /**
   * Сколько первых волн занимает трек проверок. Код начинается после него целиком —
   * ADR-0082, и это сильнее пары «проверка за своей задачей».
   */
  testWaves: number;
  /** Длина цепочки, если бы общие артефакты не мешали — цена того, как написаны задачи. */
  idealChain: number;
  depthOf: Map<string, number>;
  chainLength: number;
  peak: number;
  ready: number;
}

/**
 * Считать ли этапы последовательными.
 *
 * Два законных ответа, и разница велика: с порядком этапов у myack выходит 70
 * волн, без него — 19. `true` — план читается как выпуск: этап проходится
 * целиком, «частично» не бывает. `false` — порядок держат только объявленные
 * зависимости и общие артефакты, а номер вехи говорит лишь о принадлежности.
 * Выбор объявляется вызывающим, потому что это вопрос к плану, а не к расчёту.
 */
export interface WaveOptions {
  milestoneFloors?: boolean;
}

export function layoutWaves<T extends WaveTask>(
  tasks: readonly T[],
  { milestoneFloors = true }: WaveOptions = {},
): WaveLayout<T> {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const depthOf = new Map<string, number>();
  const depth = (id: string, seen: Set<string>): number => {
    const known = depthOf.get(id);
    if (known !== undefined) return known;
    if (seen.has(id)) return 0; // цикл: не углубляемся, гейт G3 такие ловит отдельно
    seen.add(id);
    const task = byId.get(id);
    const parents = task?.dependsOn.filter((p) => byId.has(p)) ?? [];
    const value = parents.length ? 1 + Math.max(...parents.map((p) => depth(p, seen))) : 0;
    seen.delete(id);
    depthOf.set(id, value);
    return value;
  };
  for (const task of tasks) depth(task.id, new Set());

  // Трек проверок раскладывается сам по себе; код сдвигается за него целиком.
  const isTest = (task: T) => task.kind === "test";
  const testDepths = tasks.filter(isTest).map((t) => depthOf.get(t.id) ?? 0);
  const testWaves = testDepths.length ? Math.max(...testDepths) + 1 : 0;

  // Этапы идут по номеру: задача вехи M2 не начинается, пока не пройдены волны M0 и M1.
  const milestoneNumber = (task: T) => Number(/(\d+)/.exec(task.milestoneId ?? "")?.[1] ?? 0);
  const floors = new Map<number, number>();
  let floor = 0;
  for (const number of [...new Set(tasks.map(milestoneNumber))].sort((a, b) => a - b)) {
    floors.set(number, floor);
    const inside = tasks.filter((t) => milestoneNumber(t) === number);
    floor += Math.max(...inside.map((t) => depthOf.get(t.id) ?? 0)) + 1;
  }

  const baseOf = (task: T) =>
    (depthOf.get(task.id) ?? 0) +
    (milestoneFloors ? (floors.get(milestoneNumber(task)) ?? 0) : 0) +
    (isTest(task) ? 0 : testWaves);

  const ordered = [...tasks].sort((a, b) => baseOf(a) - baseOf(b) || a.id.localeCompare(b.id));
  // Считаем так же, как итоговые волны — по непустым уровням, иначе дыры от этапов
  // сделают «идеал» больше настоящего.
  const idealChain = new Set(ordered.map(baseOf)).size;

  /**
   * Общий артефакт — та же зависимость: две задачи, правящие один экран или одну операцию
   * контракта, нельзя вести разом, даже если в плане связи между ними нет. Раскладываем
   * жадно: задача встаёт в первую волну не раньше своей, где никто не занял её артефакт.
   */
  const taken = new Map<number, Set<string>>();
  const placed = new Map<string, number>();
  for (const task of ordered) {
    const artifacts = task.artifacts ?? [];
    let wave = baseOf(task);
    for (;;) {
      const busy = taken.get(wave);
      if (!busy || !artifacts.some((a) => busy.has(a))) break;
      wave += 1;
    }
    const busy = taken.get(wave) ?? new Set<string>();
    for (const a of artifacts) busy.add(a);
    taken.set(wave, busy);
    placed.set(task.id, wave);
  }

  const grouped = new Map<number, T[]>();
  for (const task of tasks) {
    const d = placed.get(task.id) ?? 0;
    grouped.set(d, [...(grouped.get(d) ?? []), task]);
  }
  const waves = [...grouped.entries()].sort((a, b) => a[0] - b[0]).map(([, list]) => list);
  // Пока открыта хоть одна проверка, код не готов — сколько бы его зависимости ни были закрыты.
  const testsOpen = tasks.some((t) => isTest(t) && t.state !== "closed");
  return {
    waves,
    testWaves,
    idealChain,
    depthOf,
    chainLength: waves.length,
    peak: waves.reduce((n, w) => Math.max(n, w.length), 0),
    ready: tasks.filter((t) => t.state === "not_started" && t.blockedBy === 0 && (isTest(t) || !testsOpen)).length,
  };
}

/** Цепочка выбранной задачи: всё, чего она ждёт, и всё, что ждёт её. */
export function chainOf(tasks: readonly WaveTask[], id: string): Set<string> {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const chain = new Set<string>([id]);
  const up = (from: string) => {
    for (const parent of byId.get(from)?.dependsOn ?? []) {
      if (!byId.has(parent) || chain.has(parent)) continue;
      chain.add(parent);
      up(parent);
    }
  };
  const down = (from: string) => {
    for (const task of tasks) {
      if (chain.has(task.id) || !task.dependsOn.includes(from)) continue;
      chain.add(task.id);
      down(task.id);
    }
  };
  up(id);
  down(id);
  return chain;
}

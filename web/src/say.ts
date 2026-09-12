/**
 * Язык интерфейса.
 *
 * Двери харнеса отдают данные почти нейтрально — колонки сводки приходят
 * английскими именами (`area`, `priority`, `satisfied`), — а прозу они пишут
 * по-русски: причина отказа, довод правила, объяснение пустоты. Перевести
 * ЕЁ интерфейс не может и не должен: это слова, которыми владеет проект.
 *
 * Поэтому перевод накрывает подписи интерфейса, а не содержимое набора. Там,
 * где приходит проза харнеса, она показывается как есть — на любом языке.
 */
export type Lang = "ru" | "en";

const СЛОВАРЬ: Record<string, [string, string]> = {
  // Оболочка
  "nav.pult": ["Пульт", "Console"],
  "nav.pult.note": ["что идёт и что ждёт", "running and waiting"],
  "nav.where": ["Готовность", "Readiness"],
  "nav.where.note": ["фазы и гейты", "phases and gates"],
  "nav.tasks": ["Задачи", "Tasks"],
  "nav.tasks.note": ["конвейер", "pipeline"],
  "nav.corpus": ["Корпус", "Corpus"],
  "nav.corpus.note": ["всё, что знает проект", "everything the project knows"],
  "nav.depends": ["Зависимости", "Dependencies"],
  "nav.depends.note": ["что на чём стоит", "what rests on what"],
  "nav.unknown": ["Не знаем", "Unknown"],
  "nav.unknown.note": ["пробелы", "gaps"],
  "nav.read": ["Документы", "Documents"],
  "nav.read.note": ["читать", "read"],

  // Пульт
  "pu.working": ["Делают агенты", "Agents at work"],
  "pu.asks": ["От тебя ждут", "Waiting on you"],
  "pu.moved": ["Что поехало", "What shifted"],
  "pu.asks.note": ["без ответа конвейер стоит", "the pipeline stalls without an answer"],
  "pu.moved.note": ["стояли на том, что правили после них", "they rest on things edited later"],
  "pu.idle": ["Никто не занят: очередь задач пуста либо прогоны завершены.",
              "Nobody is busy: the task queue is empty or the runs have finished."],
  "pu.noasks": ["Ничего не ждут.", "Nothing is waiting."],
  "pu.noasks.why": ["Вопросов владельцу не отдано — конвейер идёт сам.",
                    "No questions are addressed to the owner — the pipeline runs on its own."],
  "pu.nomoved": ["Ничего не поехало.", "Nothing shifted."],
  "pu.nomoved.why": ["Ни одна запись не стоит на том, что изменили позже.",
                     "No record rests on anything edited after it."],
  "pu.answer": ["Ответить", "Answer"],
  "pu.collapse": ["Свернуть", "Collapse"],
  "pu.write": ["Записать ответ", "Save answer"],
  "pu.writing": ["Пишу…", "Saving…"],
  "pu.draft": ["Ответ владельца — он попадёт в вопрос и закроет ступень",
               "The owner's answer — it goes into the question and clears the step"],
  "pu.viaDoor": ["запись идёт дверью харнеса, как и всё прочее",
                 "written through a harness door, like everything else"],
  "pu.refused": ["дверь не приняла ответ", "the door refused the answer"],
  "pu.chain": ["вся цепочка →", "full chain →"],
  "pu.more": ["ещё", "another"],
  "pu.passed": ["пройдено", "passed"],
  "pu.of": ["из", "of"],
  "pu.gate": ["гейт", "gate"],
  "pu.red": ["красен", "failing"],
  "pu.green": ["зелен", "passing"],
  "pu.step": ["ступень", "step"],
  "pu.doing": ["делает", "doing"],
  "pu.attempt": ["попытка", "attempt"],
  "pu.reading": ["Читаю пульт…", "Reading the console…"],

  // Работа
  "wk.reading": ["Читаю работу…", "Reading work…"],
  "wk.closed": ["Закрыто", "Closed"],
  "wk.queue": ["в очереди предполёта", "in the preflight queue"],
  "wk.fresh": ["свежих", "fresh"],
  "wk.stale": ["протухших", "stale"],
  "wk.never": ["ни разу", "never run"],
  "wk.waves": ["Волны предполёта", "Preflight waves"],
  "wk.waveNote": ["доля свежих вердиктов в волне; щелчок оставляет только её",
                  "share of fresh verdicts per wave; click to keep only it"],
  "wk.honest": ["У задачи датирован только предполёт: закрытие помечено коммитом, а не датой — путь задачи лента показать не может.",
                "Only the preflight is dated: a closing is marked by a commit, not a date — the task's path is not something this timeline can show."],
  "wk.tasks": ["Задачи", "Tasks"],
  "wk.allWaves": ["все волны", "all waves"],
  "wk.none": ["В очереди пусто.", "The queue is empty."],
  "wk.colTask": ["задача", "task"],
  "wk.colTitle": ["название", "title"],
  "wk.colWhy": ["почему в очереди", "why queued"],
  "wk.colRev": ["правка", "revision"],
  "wk.ofUnclosed": ["из", "of"],
  "wk.unclosed": ["незакрытых", "unclosed"],
  "wk.reasons": ["Причины разложены дверью:", "The door breaks the reasons down:"],
  "wk.clickDrawer": ["Щелчок раскрывает задачу дровером, список остаётся.",
                     "A click opens the task in a drawer; the list stays put."],
  "wk.tornFoot": ["Красная грань и знак ↯ — ступень через голову:",
                  "A red edge and ↯ mark a step taken over the head:"],
  "wk.onWhole": ["на весь проект", "across the whole project"],
  "wk.allSame": ["Все одного рода: миновала", "All of one kind: skipped"],
  "wk.allIn": ["Все в", "All in"],
  "wk.noneTorn": ["Ни одна задача не проходила ступень через голову.",
                  "No task took a step over the head."],
  "wk.movedOut": ["Ступени без задач вынесены вправо; в лестнице они стоят так:",
                  "Steps with no tasks are moved to the right; in the ladder they sit like this:"],
  "wk.nth": ["-я", "th"],
  "wk.unusedHead": ["ступени без задач:", "steps with no tasks:"],
  "wk.empty": ["пусто", "empty"],
  "wk.stepsCount": ["этапов", "milestones"],
  "wk.stepsPick": ["щелчок уводит на доску с этим отбором", "a click opens the board filtered to it"],
  "nav.open": ["Открыть разделы", "Open sections"],
  "nav.close": ["Закрыть разделы", "Close sections"],
  "nav.all": ["Все проекты", "All projects"],
  "wk.viewList": ["список", "list"],
  "wk.viewBoard": ["доска", "board"],
  "wk.viewSteps": ["этапы", "milestones"],
  "wk.devTasks": ["задач разработки", "dev tasks"],
  "wk.mirrors": ["зеркал проверок", "check mirrors"],
  "wk.torn": ["со ступенью через голову", "skipped a step"],
  "wk.at": ["стоит", "here"],
  "wk.unmeasured": ["неизмеримо", "unmeasurable"],
  "wk.noFact": ["факт этой ступени никто не записывает — это «неизвестно», а не «пусто».",
                "nobody records this step's fact — that is «unknown», not «empty»."],
  "wk.transient": ["ненакопительная ступень: факт описывает состояние сейчас, и его отсутствие не значит, что задача здесь не была.",
                   "a non-accumulating step: the fact describes the state right now, and its absence does not mean the task was never here."],
  "wk.passed": ["дальше прошли все", "everyone went further"],
  "wk.missed": ["миновала", "skipped"],
  "wk.boardNote": ["каждая задача стоит в самой дальней ступени, которой достигла; красная грань — ступень через голову",
                   "each task sits at the furthest step it reached; a red edge means a step was skipped"],
  "wk.stepsNote": ["полоса этапа — доля его задач по ступеням",
                   "a milestone's bar is the share of its tasks by step"],
  "wk.mirrorOf": ["зеркало", "mirror"],
  "wk.more": ["ещё", "more"],
  "wk.idle": ["не начата", "not started"],
  "wk.freshOne": ["свеж", "fresh"],
  "wk.staleOne": ["протух", "stale"],
  "wk.neverOne": ["ни разу", "never"],

  // Корпус
  "co.reading": ["Читаю корпус…", "Reading the corpus…"],
  "co.kinds": ["Роды", "Kinds"],
  "co.rows": ["записей", "records"],
  "co.pick": ["Выберите род слева.", "Pick a kind on the left."],
  "co.none": ["Записей этого рода нет.", "There are no records of this kind."],
  "co.back": ["← ко всем", "← all records"],
  "co.proof": ["Чем доказано", "Proven by"],
  "co.rests": ["На нём стоят", "Rests on it"],
  "co.written": ["Где написано", "Written in"],
  "co.impact": ["Правка затронет", "An edit touches"],
  "co.willReopen": ["переоткроются", "will reopen"],
  "co.linked": ["связано", "linked"],
  "co.section": ["раздел", "section"],
  "co.noproof": ["нечем", "nothing"],
  "co.live.current": ["свежо", "current"],
  "co.live.reopened": ["переоткрыто", "reopened"],
  "co.search": ["Искать по имени", "Search by name"],
  "co.cols": ["колонок", "more columns"],
  "co.fold": ["свернуть колонки", "fold columns"],
  "co.groupBy": ["собрать по", "group by"],

  // Зависимости
  "de.skeleton": ["Скелет", "Skeleton"],
  "de.kinds": ["Роды", "Kinds"],
  "de.what": ["Что переоткроется", "What will reopen"],
  "de.reading": ["Читаю связи…", "Reading the links…"],
  "de.rule": ["значит «А стоит на Б»: правка Б переоткрывает А. Толщина — сколько связей.",
              "means “A rests on B”: editing B reopens A. Thickness is the number of links."],
  "de.pickKind": ["Выберите род на карте.", "Pick a kind on the map."],
  "de.pickRow": ["— выберите запись —", "— pick a record —"],
  "de.on": ["стоят:", "rest on it:"],
  "de.nobody": ["никто — правка не переоткроет ничего", "nobody — an edit reopens nothing"],
  "de.touched": ["затронуто", "touched"],
  "de.ofThem": ["из них переоткроется", "of those will reopen"],
  "de.step": ["шаг", "step"],
  "de.willReopen": ["переоткроется", "will reopen"],
  "de.linked": ["связано", "linked"],
  "de.noOne": ["На этой записи не стоит никто: правка никого не затронет.",
               "Nothing rests on this record: an edit touches nobody."],
  "de.reopens": ["Переоткрываются при правке связей:", "Reopen when their links change:"],
  "de.noReopens": ["ни один", "none"],
  "de.notReopen": ["не переоткрывается", "does not reopen"],
  "de.reopened": ["переоткрыто", "reopened"],
  "de.byDomain": ["по областям", "by domain"],
  "de.byKind": ["по родам", "by kind"],
};

/** Названия родов. Имя вида — слово проекта; перевод лишь подписывает его. */
const РОДЫ: Record<string, [string, string]> = {
  requirement: ["требование", "requirement"],
  check: ["проверка", "check"],
  story: ["история", "story"],
  task: ["задача", "task"],
  "red-task": ["красная задача", "red task"],
  milestone: ["этап", "milestone"],
  question: ["вопрос", "question"],
  screen: ["экран", "screen"],
  need: ["потребность", "need"],
  decision: ["решение", "decision"],
  rationale: ["рассуждение", "rationale"],
  "lint-rule": ["правило кода", "lint rule"],
  assertion: ["утверждение", "assertion"],
  term: ["термин", "term"],
  article: ["статья", "article"],
  feature: ["фича", "feature"],
  version: ["выпуск", "release"],
  run: ["прогон", "run"],
  risk: ["риск", "risk"],
  gate: ["гейт", "gate"],
};

/** Области проекта. Имя области — слово проекта; перевод его подписывает. */
const ОБЛАСТИ: Record<string, string> = {
  "требования": "requirements",
  "доказательство": "proof",
  "план": "plan",
  "знание": "knowledge",
  "надзор": "oversight",
};

const КЛЮЧ = "mh-lang";

export function langNow(): Lang {
  const url = new URL(window.location.href).searchParams.get("lang");
  if (url === "en" || url === "ru") return url;
  try {
    const kept = localStorage.getItem(КЛЮЧ);
    if (kept === "en" || kept === "ru") return kept;
  } catch {
    // Хранилище бывает закрыто — язык тогда просто не запоминается.
  }
  return "ru";
}

export function setLang(l: Lang): void {
  try {
    localStorage.setItem(КЛЮЧ, l);
  } catch {
    // см. выше
  }
  const url = new URL(window.location.href);
  url.searchParams.set("lang", l);
  window.history.replaceState({}, "", url);
  window.dispatchEvent(new Event("mh-lang"));
}

/**
 * Подпись по ключу. Ключа нет — возвращается сам ключ: пропущенный перевод
 * ВИДЕН, а не подменён пустотой. Молчание здесь читалось бы как «так и надо».
 */
export function say(l: Lang, key: string): string {
  const пара = СЛОВАРЬ[key];
  if (!пара) return key;
  return l === "en" ? пара[1] : пара[0];
}

/** Как зовут область. Необъявленная показывается своим именем. */
export function domainName(l: Lang, d: string): string {
  return l === "en" ? (ОБЛАСТИ[d] ?? d) : d;
}

/** Как зовут род на этом языке. Неизвестный род показывается своим именем. */
export function kindName(l: Lang, kind: string): string {
  const пара = РОДЫ[kind];
  if (!пара) return kind;
  return l === "en" ? пара[1] : пара[0];
}

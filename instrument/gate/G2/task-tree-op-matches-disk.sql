WITH свежий AS (
  SELECT k FROM unnest(ARRAY['repo-file', 'code-file']) AS k WHERE fact_fresh($1, k)),
датчик AS (
  SELECT EXISTS (SELECT 1 FROM свежий) AS свеж),
лист AS (
  SELECT l.task_id, l.op, l.path,
         CASE WHEN right(l.path, 1) = '/'
              THEN EXISTS (SELECT 1 FROM code_fact f WHERE f.project_id = l.project_id AND f.kind IN (SELECT k FROM свежий)
                            AND left(f.name, length(l.path)) = l.path)
              ELSE EXISTS (SELECT 1 FROM code_fact f WHERE f.project_id = l.project_id AND f.kind IN (SELECT k FROM свежий)
                            AND f.name = l.path) END AS на_диске
    FROM project_task_tree_leaf l
    JOIN project_plan_tasks t ON t.project_id = l.project_id AND t.id = l.task_id AND t.state <> 'closed'
   WHERE l.project_id = $1 AND l.is_path AND NOT l.exempt AND l.path <> '' AND l.op <> ''
     -- ЗАДАЧА В РАБОТЕ НЕ СУДИТСЯ. Её пометки описывают ПЕРЕХОД, а не
     -- состояние: «+» значит «заведу», и заведённый файл делает пометку
     -- «неверной» ровно в тот миг, когда работа пошла. «-» — то же зеркально.
     --
     -- Петля была замкнутой: этот пункт держит `G2`, `G2` открывает фазу, фаза
     -- пускает работу — а работа красит пункт первым же созданным файлом.
     -- Выходило «фаза открыта, только когда никто не работает», и единственный
     -- выход из петли — закрыться при закрытой фазе, что пишет долг `task_redo`.
     -- Так набор и получил 81 строку долга.
     --
     -- «В работе» берётся у открытого рабочего дерева — факта, а не намерения.
     -- Задача в покое судится как прежде: «+» на существующий файл — настоящая
     -- находка, кто-то уже построил обещанное.
     AND NOT EXISTS (SELECT 1 FROM task_worktree w
                      WHERE w.project_id = l.project_id AND w.task_id = l.task_id)),
предок AS (
  WITH RECURSIVE до(task_id, dep) AS (
    SELECT d.task_id, d.depends_on FROM project_plan_task_deps d WHERE d.project_id = $1
    UNION
    SELECT д.task_id, d.depends_on FROM до д JOIN project_plan_task_deps d ON d.project_id = $1 AND d.task_id = д.dep)
  SELECT task_id, dep FROM до)
SELECT 'датчик файлов дерева «repo-file» ' || fact_gap($1, 'repo-file')
       || ': сверить пометки «+ ! -» задач с деревом нечем, и это не зелёное' AS detail
  FROM датчик WHERE NOT свеж AND EXISTS (SELECT 1 FROM лист)
UNION ALL
SELECT л.task_id || ' — «+ ' || л.path || '»: файл уже есть в дереве — пометка «!»'
  FROM лист л, датчик WHERE датчик.свеж AND л.op = '+' AND л.на_диске AND л.path NOT LIKE '%/'
UNION ALL
SELECT л.task_id || ' — «' || л.op || ' ' || л.path || '»: файла нет, и ни одна зависимость задачи его не создаёт'
  FROM лист л, датчик
 WHERE датчик.свеж AND л.op IN ('!', '-') AND NOT л.на_диске
   AND NOT EXISTS (SELECT 1 FROM предок п
                     JOIN project_plan_tasks pt ON pt.project_id = $1 AND pt.id = п.dep AND pt.state <> 'closed'
                     JOIN project_task_tree_leaf pl ON pl.project_id = $1 AND pl.task_id = п.dep AND pl.op = '+' AND pl.path = л.path
                    WHERE п.task_id = л.task_id)
   -- ЗЕРКАЛО СОЗДАЁТ ТО, ЧТО ПАРА УБИРАЕТ, И ЗАВИСИМОСТЬЮ ОНО НЕ БЫВАЕТ.
   --
   -- Пара пишет «- mirror_X.rs»: зеркало заведёт файл, пара напишет тело и
   -- переименует. Прощение выше ищет создателя среди ЗАВИСИМОСТЕЙ, а зеркало
   -- задачи — не зависимость: связь идёт полем «Пара», и объявить её
   -- зависимостью дверь отказывает — строка красной задачи пересобирается из
   -- документа.
   --
   -- Цена была не в лишней строке, а в остановке: пункт держит `G2`, `G2`
   -- открывает фазу Ф4, а в ней идёт вся работа. Значит ЗАВЕДЕНИЕ НОВОЙ ПАРЫ
   -- останавливало очередь задач — от заведения до посадки зеркала. Замер
   -- 21.09: так вышло дважды за день, у `M5-T33` и у `M4-T9`, и оба раза
   -- очередь стояла, пока зеркало не село.
   --
   -- Зеркало прощает на тех же условиях, что зависимость: оно не закрыто (а
   -- закрытое обязано было файл оставить) и объявляет этот самый путь «+».
   AND NOT EXISTS (SELECT 1 FROM red_task r
                     JOIN project_plan_tasks rt
                       ON rt.project_id = $1 AND rt.id = r.id AND rt.state <> 'closed'
                     JOIN project_task_tree_leaf rl
                       ON rl.project_id = $1 AND rl.task_id = r.id AND rl.op = '+' AND rl.path = л.path
                    WHERE r.project_id = $1 AND lower(r.parent_task) = lower(л.task_id))
 ORDER BY 1

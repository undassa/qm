SELECT d.task_id || ' → ' || d.depends_on || ': такой задачи нет' AS detail FROM project_plan_task_deps d WHERE d.project_id = $1 AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t WHERE t.project_id = d.project_id AND t.id = d.depends_on)
   -- НОМЕРА ЗАДАЧ ВНУТРИ ВЕХИ НЕ СРАВНИВАЮТСЯ. Номер — порядок заведения, а
   -- не исполнения: при плотной нумерации `M5` задача, решённая позже плана
   -- (`M5-T94`, нужная `M5-T88`), получала только больший номер, и её
   -- зависимость краснела без выхода (undassa/mh#124). Невыполнимый порядок
   -- внутри вехи — это круг, а его ловит последнее слагаемое.
UNION ALL
SELECT d.task_id || ' → ' || d.depends_on || ': зависит от задачи более поздней вехи ' || mb.id
  FROM project_plan_task_deps d
  JOIN project_plan_tasks a ON a.project_id = d.project_id AND a.id = d.task_id
  JOIN project_plan_tasks b ON b.project_id = d.project_id AND b.id = d.depends_on
  JOIN project_plan_milestones ma ON ma.project_id = a.project_id AND ma.id = a.milestone_id
  JOIN project_plan_milestones mb ON mb.project_id = b.project_id AND mb.id = b.milestone_id AND a.kind = b.kind
 WHERE d.project_id = $1 AND mb.ord > ma.ord
   -- НОМЕР ВЕХИ СРАВНИВАЕТСЯ ТОЛЬКО ВНУТРИ ТРЕКА. Зеркальный трек идёт
   -- ПАРАЛЛЕЛЬНО дев-треку, а не после него: у `tot-ade` веха `V3` занимает
   -- волны 1..12, а её пара `M3` — 8..14, и так у всех семи пар. Порядок вех
   -- между треками не выражает ничего, и всякая объявленная зависимость
   -- дев-задачи на своё зеркало называлась обратной ложно.
   --
   -- Судить по волне вместо номера нельзя: волна ВЫВОДИТСЯ из зависимостей, и
   -- «смотрит назад по волне» было бы верно всегда — проверка, зелёная по
   -- построению. Номер вехи остаётся, он написан рукой и потому может
   -- разойтись с зависимостями; сравнивается он там, где сравним.
   --
   -- Круг ловится отдельным слагаемым ниже, и он-то поперёк треков и работает.
UNION ALL
SELECT c.task_id || ' — зависимость по кругу: задача ждёт саму себя' FROM (
  WITH RECURSIVE доходит(task_id, до) AS (
    SELECT d.task_id, d.depends_on FROM project_plan_task_deps d WHERE d.project_id = $1
    UNION
    SELECT д.task_id, d.depends_on FROM доходит д
      JOIN project_plan_task_deps d ON d.project_id = $1 AND d.task_id = д.до)
  SELECT task_id FROM доходит WHERE task_id = до) c
 ORDER BY 1

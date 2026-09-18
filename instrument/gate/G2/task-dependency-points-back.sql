SELECT d.task_id || ' → ' || d.depends_on || ': такой задачи нет' AS detail FROM project_plan_task_deps d WHERE d.project_id = $1 AND NOT EXISTS (SELECT 1 FROM project_plan_tasks t WHERE t.project_id = d.project_id AND t.id = d.depends_on) UNION ALL SELECT d.task_id || ' → ' || d.depends_on || ': зависит от более поздней' FROM project_plan_task_deps d JOIN project_plan_tasks a ON a.project_id = d.project_id AND a.id = d.task_id JOIN project_plan_tasks b ON b.project_id = d.project_id AND b.id = d.depends_on WHERE d.project_id = $1 AND a.milestone_id = b.milestone_id AND b.number > a.number
UNION ALL
SELECT d.task_id || ' → ' || d.depends_on || ': зависит от задачи более поздней вехи ' || mb.id
  FROM project_plan_task_deps d
  JOIN project_plan_tasks a ON a.project_id = d.project_id AND a.id = d.task_id
  JOIN project_plan_tasks b ON b.project_id = d.project_id AND b.id = d.depends_on
  JOIN project_plan_milestones ma ON ma.project_id = a.project_id AND ma.id = a.milestone_id
  JOIN project_plan_milestones mb ON mb.project_id = b.project_id AND mb.id = b.milestone_id
 WHERE d.project_id = $1 AND mb.ord > ma.ord
UNION ALL
SELECT c.task_id || ' — зависимость по кругу: задача ждёт саму себя' FROM (
  WITH RECURSIVE доходит(task_id, до) AS (
    SELECT d.task_id, d.depends_on FROM project_plan_task_deps d WHERE d.project_id = $1
    UNION
    SELECT д.task_id, d.depends_on FROM доходит д
      JOIN project_plan_task_deps d ON d.project_id = $1 AND d.task_id = д.до)
  SELECT task_id FROM доходит WHERE task_id = до) c
 ORDER BY 1

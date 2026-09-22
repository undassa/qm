-- ПРЕДМЕТ — пункты готовности, чей инструмент вообще грепп. Набор, где таких
-- нет, не «чист», а не даёт правилу предмета: об этом говорит `subjectWhy`.
SELECT 1
  FROM task_ready_item r
 WHERE r.project_id = $1
   AND r.text ~ '(?:^|[^[:alnum:]])(?:rg|grep)[[:space:]]'
 LIMIT 1

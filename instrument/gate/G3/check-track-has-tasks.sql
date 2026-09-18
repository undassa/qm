SELECT 'трек проверок пуст' AS detail WHERE NOT EXISTS (
              SELECT 1 FROM project_plan_tasks WHERE project_id=$1 AND kind='red')

INSERT INTO project_plan_task_deps(project_id, task_id, depends_on) SELECT $1, min(id), 'M9-T99' FROM project_plan_tasks WHERE project_id=$1

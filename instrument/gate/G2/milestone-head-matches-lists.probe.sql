INSERT INTO project_milestone_count(project_id, milestone_id, what, said, listed) SELECT $1, min(milestone_id), 'проба', 7, 9 FROM project_plan_tasks WHERE project_id=$1

INSERT INTO project_phase_artifact(project_id, phase, phase_id, artifact, we_have, state) SELECT $1, 'Фаза 2', p.id, 'проба самотеста', '', 'нет' FROM phase p WHERE p.gate='G2' LIMIT 1

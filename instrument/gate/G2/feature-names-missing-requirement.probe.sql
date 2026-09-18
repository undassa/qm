INSERT INTO project_feature_requirements(project_id, feature_id, requirement_id, origin) SELECT $1, min(feature_id), 'FR-НЕТ-99', 'declared' FROM project_feature_requirements WHERE project_id=$1

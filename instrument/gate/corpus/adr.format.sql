WITH имена(предмет, заголовок) AS (VALUES ('Контекст', 'Контекст'), ('Контекст', 'Context'), ('Решение', 'Решение'), ('Решение', 'Decision'),
  ('Отвергнутые варианты', 'Отвергнутые варианты'), ('Отвергнутые варианты', 'Alternatives considered'), ('Последствия', 'Последствия'), ('Последствия', 'Consequences')),
канон(основа, заголовок) AS (VALUES ('отвергн', 'Отвергнутые варианты'), ('решени', 'Решение'), ('контекст', 'Контекст'), ('последстви', 'Последствия'), ('гейт', 'Гейт'),
  ('alternative', 'Alternatives considered'), ('context', 'Context'), ('decision', 'Decision'), ('consequence', 'Consequences')),
заголовки AS (SELECT s.entity_name, s.title FROM project_document_sections s WHERE s.project_id = $1 AND s.entity_kind = 'decision' AND s.level = 2)
SELECT р.entity_name || ' — нет раздела «' || п.предмет || '»' AS detail FROM project_decisions р CROSS JOIN (SELECT DISTINCT предмет FROM имена) п WHERE р.project_id = $1 AND NOT EXISTS (SELECT 1 FROM заголовки з JOIN имена и ON и.заголовок = з.title WHERE з.entity_name = р.entity_name AND и.предмет = п.предмет)
UNION ALL
SELECT з.entity_name || ' — «' || з.title || '» — второе имя для «' || к.заголовок || '»' FROM заголовки з JOIN канон к ON left(lower(з.title), length(к.основа)) = к.основа WHERE з.title <> к.заголовок AND з.title NOT IN (SELECT заголовок FROM имена)
ORDER BY 1

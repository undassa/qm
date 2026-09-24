WITH имена(статус, предмет, заголовок) AS (VALUES ('accepted', 'Контекст', 'Контекст'), ('accepted', 'Контекст', 'Context'), ('accepted', 'Решение', 'Решение'), ('accepted', 'Решение', 'Decision'),
  ('accepted', 'Отвергнутые варианты', 'Отвергнутые варианты'), ('accepted', 'Отвергнутые варианты', 'Alternatives considered'), ('accepted', 'Последствия', 'Последствия'), ('accepted', 'Последствия', 'Consequences'),
  ('proposed', 'Контекст', 'Контекст'), ('proposed', 'Контекст', 'Context'), ('proposed', 'Предложение', 'Предложение'), ('proposed', 'Предложение', 'Proposal'),
  ('proposed', 'Отвергнутые варианты', 'Отвергнутые варианты'), ('proposed', 'Отвергнутые варианты', 'Alternatives considered'), ('proposed', 'Последствия', 'Последствия'), ('proposed', 'Последствия', 'Consequences')),
канон(основа, заголовок) AS (VALUES ('отвергн', 'Отвергнутые варианты'), ('решени', 'Решение'), ('контекст', 'Контекст'), ('последстви', 'Последствия'), ('гейт', 'Гейт'), ('предложени', 'Предложение'),
  ('alternative', 'Alternatives considered'), ('context', 'Context'), ('decision', 'Decision'), ('consequence', 'Consequences'), ('proposal', 'Proposal')),
решения AS (SELECT р.entity_name, CASE WHEN р.status = 'proposed' THEN 'proposed' ELSE 'accepted' END AS вид FROM project_decisions р WHERE р.project_id = $1),
заголовки AS (SELECT s.entity_name, s.title FROM project_document_sections s WHERE s.project_id = $1 AND s.entity_kind = 'decision' AND s.level = 2)
SELECT р.entity_name || ' — нет раздела «' || п.предмет || '»' AS detail FROM решения р JOIN (SELECT DISTINCT статус, предмет FROM имена) п ON п.статус = р.вид WHERE NOT EXISTS (SELECT 1 FROM заголовки з JOIN имена и ON и.заголовок = з.title AND и.статус = р.вид WHERE з.entity_name = р.entity_name AND и.предмет = п.предмет)
UNION ALL
SELECT з.entity_name || ' — «' || з.title || '» — второе имя для «' || к.заголовок || '»' FROM заголовки з JOIN канон к ON left(lower(з.title), length(к.основа)) = к.основа WHERE з.title <> к.заголовок AND з.title NOT IN (SELECT заголовок FROM имена)
ORDER BY 1

SELECT 'пересборка проекций не удалась: статьи конституции не прочитаны' AS entity WHERE NOT EXISTS (SELECT 1 FROM reproject_state r WHERE r.project_id = $1 AND r.ok)
UNION ALL
SELECT 'статья ' || a.number || ' · ' || a.title FROM project_articles a WHERE a.project_id = $1
UNION ALL
SELECT 'ART-' || lpad(g.article::text, 2, '0') || ' ⇄ ' || g.gate FROM project_article_gates g WHERE g.project_id = $1

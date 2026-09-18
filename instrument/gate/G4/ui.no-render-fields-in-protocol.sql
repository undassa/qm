WITH протокол AS (
  SELECT DISTINCT каталог FROM (
    SELECT DISTINCT ON (p.name) regexp_replace(m.name, 'Cargo\.toml$', '') AS каталог
      FROM code_fact p JOIN code_fact m ON m.project_id = p.project_id AND m.kind = 'crate-manifest'
       AND left(p.name, length(regexp_replace(m.name, 'Cargo\.toml$', ''))) = regexp_replace(m.name, 'Cargo\.toml$', '')
     WHERE p.project_id = $1 AND p.kind = 'protocol-item' AND p.detail ~ '^pub enum (Op|EventMsg)\y'
     ORDER BY p.name, length(m.name) DESC) x)
SELECT 'датчик «render-field» ' || fact_gap($1, 'render-field') || ': есть ли поля отрисовки — неизвестно' AS detail WHERE NOT fact_fresh($1, 'render-field')
UNION ALL
SELECT 'датчик «protocol-item» ' || fact_gap($1, 'protocol-item') || ': прочитан ли протокол — неизвестно' AS detail WHERE NOT fact_fresh($1, 'protocol-item')
UNION ALL
SELECT 'датчик «crate-manifest» ' || fact_gap($1, 'crate-manifest') || ': какому крейту принадлежит протокол — неизвестно' AS detail WHERE NOT fact_fresh($1, 'crate-manifest')
UNION ALL
SELECT 'протокол — крейта с enum Op и EventMsg не найдено: модулей протокола не прочитано' WHERE NOT EXISTS (SELECT 1 FROM протокол)
UNION ALL
SELECT f.name || ' — поле отрисовки в протоколе: «' || f.detail || '»' FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'render-field' AND EXISTS (SELECT 1 FROM протокол п WHERE left(f.name, length(п.каталог || 'src/')) = п.каталог || 'src/')
ORDER BY 1

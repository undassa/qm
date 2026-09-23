-- «НЕТ В ДЕРЕВЕ» ПРАВИЛО СКАЗАТЬ НЕ МОЖЕТ, И БОЛЬШЕ НЕ ГОВОРИТ. Сценарий
-- ищется среди имён, ПОДАННЫХ датчиком, а не на диске: файл вне охвата
-- датчика неотличим здесь от несуществующего. Третий пункт того же рода,
-- что `task-path-exists` и `task-tree-op-matches-disk`; каталога назвать
-- нечем — требование даёт одно имя файла без пути, — поэтому здесь
-- правится только утверждение, а развилки по верхнему каталогу нет.
WITH сцен AS (SELECT DISTINCT r.id AS nfr, x[1] AS файл FROM project_requirements r CROSS JOIN LATERAL regexp_matches(r.measured_by, '([A-Za-z0-9._-]+[.](js|ts|mjs|py|sh|sql))\M', 'g') AS x WHERE r.project_id = $1) SELECT 'датчик файлов ' || fact_gap($1, 'repo-file') || ': есть ли сценарий — неизвестно' AS detail WHERE EXISTS (SELECT 1 FROM сцен) AND NOT fact_fresh($1, 'repo-file') UNION ALL SELECT с.nfr || ' → ' || с.файл || ': способ измерения назван сценарием, которого нет среди прочитанного датчиком. Есть ли он в дереве — набор не мерил: проверьте перечень чтения (`sensor-specs`)' FROM сцен с WHERE fact_fresh($1, 'repo-file') AND NOT EXISTS (SELECT 1 FROM code_fact f WHERE f.project_id = $1 AND f.kind IN ('repo-file','code-file') AND f.name LIKE '%' || с.файл) AND true ORDER BY 1

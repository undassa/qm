WITH строки AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н, trim(f.detail) AS т
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'workspace-lint'),
заголовки AS (
  SELECT файл, н, substring(т from '^\[workspace\.lints\.([a-z]+)\]') AS группа FROM строки WHERE т ~ '^\['),
уровни AS (
  SELECT (SELECT з.группа FROM заголовки з WHERE з.файл = с.файл AND з.н < с.н ORDER BY з.н DESC LIMIT 1) AS группа,
         substring(с.т from '^([a-z_]+)') AS линт,
         coalesce(substring(с.т from '^[a-z_]+\s*=\s*"([a-z]+)"'), substring(с.т from 'level\s*=\s*"([a-z]+)"')) AS уровень
    FROM строки с WHERE с.т !~ '^\['),
основа AS (
  SELECT s.value AS запись, substring(s.value from '^([a-z]+)::') AS группа, substring(s.value from '::([a-z_]+)') AS линт,
         substring(s.value from ' (allow|warn|deny|forbid)$') AS уровень
    FROM scheme($1) s WHERE s.role = 'lint.floor'),
ранг(уровень, р) AS (VALUES ('allow', 0), ('warn', 1), ('deny', 2), ('forbid', 3))
SELECT 'датчик «workspace-lint» ' || fact_gap($1, 'workspace-lint') || ': ослаблены ли линты — неизвестно' AS detail WHERE NOT fact_fresh($1, 'workspace-lint')
UNION ALL
SELECT 'базовый набор линтов не объявлен: у воркспейса ' || count(*) || ' уровней, а сверять их не с чем — объявите каждый дверью scheme-term-set role=lint.floor value="группа::линт уровень"' FROM уровни WHERE NOT EXISTS (SELECT 1 FROM основа) HAVING count(*) > 0
UNION ALL
SELECT 'lint.floor — «' || о.запись || '» не разобрано: ожидается «группа::линт уровень»' FROM основа о WHERE о.запись !~ '^[a-z]+::[a-z_]+ (allow|warn|deny|forbid)$'
UNION ALL
SELECT о.группа || '::' || о.линт || ' — ' || CASE WHEN у.уровень IS NULL THEN 'исчез из линтов воркспейса' ELSE 'ослаблен: ' || о.уровень || ' → ' || у.уровень END FROM основа о JOIN ранг ро ON ро.уровень = о.уровень LEFT JOIN уровни у ON у.группа = о.группа AND у.линт = о.линт LEFT JOIN ранг ру ON ру.уровень = у.уровень WHERE у.уровень IS NULL OR coalesce(ру.р, -1) < ро.р
ORDER BY 1

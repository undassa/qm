WITH границы AS (
  SELECT substring(f.name from '^(.*):\d+$') AS файл, substring(f.name from ':(\d+)$')::int AS н,
         substring(f.detail from '^pub (?:enum|struct) (\w+)') AS тип
    FROM code_fact f WHERE f.project_id = $1 AND f.kind = 'protocol-item'),
протокол AS (
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
-- ИСКЛЮЧЕНИЕ ПО ТИПУ, А НЕ ПО ИМЕНИ ПОЛЯ, и это существенно: имена `width`,
-- `height`, `color`, `pixels` пункт обязан ловить дальше — ради них он и написан.
-- Исключается тип, внутри которого поле объявлено.
--
-- Запрет статьи — про РЕШЕНИЕ ЯДРА, обязательное для поверхности. Решение
-- владельца 2026-09-23 назвало два случая, где такого решения нет. Первое:
-- данные программы — знаки, их цвета, ширина знака в ячейках, картинка с
-- пикселями; ядро здесь курьер, и поверхность показывает это не потому, что ядро
-- велело, а потому, что иначе покажет не то, что вывела программа. Второе: мерка
-- ячейки идёт ОТ поверхности к ядру, каждая поверхность шлёт свою, а которой
-- картинки не нужны — не шлёт никакой. Довод «второй рендер обязан исполнять»
-- здесь не о чем.
--
-- Типы объявляет НАБОР ролями `type.program-data` и `type.surface-measurement`.
--
-- Ближайшая граница берётся ЛЮБАЯ, без отбора по непустому типу: если ближайшая
-- сверху — не объявление типа, `тип` выходит `NULL`, и поле НЕ исключается.
-- Отказ падает в сторону красного, а не зелёного.
SELECT f.name || ' — поле отрисовки в протоколе: «' || f.detail || '»' FROM code_fact f
 WHERE f.project_id = $1 AND f.kind = 'render-field'
   AND EXISTS (SELECT 1 FROM протокол п WHERE left(f.name, length(п.каталог || 'src/')) = п.каталог || 'src/')
   AND NOT EXISTS (
     SELECT 1 FROM scheme($1) s
      WHERE s.role IN ('type.program-data', 'type.surface-measurement')
        AND s.value = (SELECT б.тип FROM границы б
                        WHERE б.файл = substring(f.name from '^(.*):\d+$')
                          AND б.н < substring(f.name from ':(\d+)$')::int
                        ORDER BY б.н DESC LIMIT 1))
ORDER BY 1

# Продолжение работы на этой машине

> **Состояние на 20 сентября 2026.** Воркер пульта — на Rust: `mh-worker`
> (`server-rs/target/release/mh-worker`, служба `mh-runner.service`), цикл 5 с,
> advisory lock в Postgres против второго экземпляра, двери зовёт в-процессе.
> Прежний `scripts/runner.py` снят. Автомат задачи — в базе: таблица
> `task_run_automaton`, дверь `run-automaton` (вердикты PLAN/GO/RETHINK/NEEDS
> FIX/DRIFT/CLOSED; капы: RETHINK×2 и пятый круг; `resume=1` снимает остановку
> по слову владельца; DRIFT встаёт с префиксом «DRIFT: » и останавливает
> конвейер целиком). Пункты приёмки задач — таблица `task_ready_item`
> (посстрочный разбор `- [ ]`/`- [x]` из документов задач), дверь `ready`,
> пункт прибора `ready-items-checked` пока в фазе corpus (хвост закрытых задач
> с неотмеченными пунктами — работа набора; перевод в G4 — после разбора).
> Одобрение коммита привязано к дереву: голова на момент approval-ask лежит в
> событии «одобрение», дерево, уехавшее после одобрения, требует нового слова.
> Прибор — 166 пунктов, 7 гейтов. Известные красные myack (протухшие датчики
> и 4 предмета-пустышки: render:blocks-from-library, page:derived-not-stored,
> ui:no-domain-types-in-surface, model:interpretation-pinned) — картина набора,
> не харнеса.

> **Читать с оглядкой на дату.** Ниже — состояние на 2 сентября 2026, и часть его
> устарела: там поверхностью назван `mh-ui`, а вход описан через портал и
> плагины. С тех пор харнес переехал в один двоичный файл на Rust, и правда
> теперь такая (18 сентября 2026):
>
> * служба одна — `mh-web.service`, `server-rs/target/release/mh-server`, порт
>   8096. Но `mh-ui.service` и `mh.service` тоже числятся `enabled`, и первая
>   объявляет ТОТ ЖЕ порт 8096. Сегодня они мертвы и не мешают; при следующей
>   загрузке машины поднимутся оба, и кто займёт порт — жребий: порядок между
>   `mh-ui` и `mh-web` не объявлен. Решение владельца — issue #25;
> * **прибор объявлен репозиторием**: `instrument/` держит 160 пунктов гейта, 7
>   гейтов, 6 фаз, 13 ступеней лестницы и раскладку 60 видов. Правится он
>   коммитом и выкладкой; дверей правки прибора на живом нет ни одной;
> * схема заводится сама — 150 таблиц объявлены сервером, и прогон в CI
>   поднимает её на пустой базе и раскладывает по ней весь прибор;
> * выкладка: `cargo build --release --locked` в `server-rs`, затем
>   `sudo -n systemctl restart mh-web.service`; перед этим — дождаться зелёного
>   прогона, снять замер обоих наборов и предупредить их сессии;
> * заявки сессий на изменение харнеса — issues `undassa/mh` с меткой
>   `harness-request`; решения владельца записаны комментариями там же (#18—#24);
> * снимки базы — `/opt/backup/mh-db`, часовые за сутки и суточные за месяц. Это
>   единственное, чем восстанавливается то, что живёт только в Postgres:
>   проверено 18 сентября, когда я потерял часть раскладки видов.
>
> ВАЖНО: прибор вшивается в бинарь на сборке (`include_str` через build.rs) —
> правка `instrument/` требует пересборки release, одного рестарта мало.

Записано 2026-09-02. Всё ниже проверено запуском, а не памятью.

## Что сейчас работает

```
mh            :8100   ядро,      Postgres, systemd, enabled
mh-ui         :8096   поверхность (Lit, plugins/web-ui)
mh-portal     :8097   ПОРТАЛ — входная дверь, проксирует поверхность
caddy                 TLS + basic_auth на code.undassa.com
mh-pg                 docker, postgres:16-alpine, 127.0.0.1:5455, restart=unless-stopped
meta-harness  :7800   доска надзирателя (временная, отпадёт — см. HARNESS.md)
```

Порядок такой: **браузер → портал :8097 → поверхность :8096 → ядро :8100**.
Поверхность сама никого не аутентифицирует и честно об этом пишет.

Все порты закрыты снаружи правилами `ufw` (7800, 8096, 8097, 8100) — переживают
перезагрузку. Наружу только Caddy.

## Как зайти сейчас

С рабочей машины:

```bash
ssh -N -L 8097:127.0.0.1:8097 workstation.undassa.local
```

и открыть **`http://localhost:8097/`** — схему писать обязательно, иначе браузер
принимает `localhost:8097` за поисковый запрос и уводит на `www.localhost.com`.
Портал редиректов не делает: проверено, `num_redirects: 0`.

Вход — `PORTAL_LOCAL_AUTH_BYPASS=1`, принципал `undassa`. Это **обход**, а не
аутентификация; безопасен ровно потому, что портал слушает loopback и доступен
только через туннель.

Права админа настоящие: грант лежит в ядре.

```sql
select * from admin_grants;   -- undassa@undassa | org:undassa | org_admin
```

Проверить вход в API напрямую:

```bash
cd /opt/src/github.com/undassa/mh
node mh-login.mjs .env undassa                                  # whoami
node mh-login.mjs .env undassa GET /v1/admin/scopes
node mh-login.mjs .env undassa GET "/v1/admin/runs?scope=org:undassa"
```

## Репозитории здесь

```
/opt/src/github.com/undassa/mh            форк, ветка harness
    origin   → git@github.com:undassa/mh.git      пишем сюда
    upstream → git@github.com:undassa/mh.git  push DISABLED
    HARNESS.md — план переработки, шесть шагов

/opt/src/github.com/undassa/meta-harness  прототип надзирателя и проекции
    origin → /opt/src/git/meta-harness.git (голый, локальный)
    теги v0.0.1 … v0.5.0, MH.md — рунбук подъёма mh
```

## Незакрытое, по важности

### 1. Домен не пускает — и это главное

`code.undassa.com` ведёт на **поверхность**, а должен на **портал**. Но просто
переключить нельзя: на домене обход не работает по построению — проверка
`isLocalPortalUrl(PUBLIC_URL)` в `plugins/portal/src/index.ts`.

Два пути, и выбрать надо один.

**Путь А — почта, без правки кода.** Брокер `plugins/auth` — OIDC-сервер с
одноразовой ссылкой. Транспорта два, `smtp` или `resend`, **без ключей не
стартует**; режима «написать ссылку в лог» нет. Нужно: `RESEND_API_KEY` либо
`SMTP_HOST/PORT/USER/PASSWORD`, `AUTH_EMAIL_FROM`, `AUTH_ALLOWED_EMAILS`.
Дальше — `AUTH_BROKER_UPSTREAM` в портале, `PORTAL_PUBLIC_URL=https://code.undassa.com`,
и корень домена на 8097.

**Путь Б — правка форка.** Портал принимает личность от Caddy, который человека
уже аутентифицировал. Патч ниже. Я его не применял: он трогает путь
аутентификации, и это стоит посмотреть глазами.

### Патч пути Б

В `plugins/portal/src/index.ts`, рядом с `LOCAL_AUTH_PRINCIPAL`:

```ts
// ФОРК. Личность от доверенного обратного прокси.
// Три условия, и все обязательны: режим включён явно; соединение пришло
// с loopback, то есть от прокси на этой же машине; прокси ПЕРЕЗАПИСЫВАЕТ
// заголовок (в Caddy — header_up), поэтому клиент его не подделает.
// Без второго заголовок подделает кто угодно, без третьего — клиент прокси.
const TRUSTED_PROXY_HEADER = (process.env.PORTAL_TRUSTED_PROXY_HEADER ?? "").trim().toLowerCase();

function trustedProxySession(req: IncomingMessage, nowMs = Date.now()): SessionClaims | null {
  if (!TRUSTED_PROXY_HEADER) return null;
  if (!isLoopbackAddress(req.socket.remoteAddress)) return null;
  const raw = req.headers[TRUSTED_PROXY_HEADER];
  const principal = (Array.isArray(raw) ? raw[0] : raw)?.trim();
  if (!principal || !/^[A-Za-z0-9._@-]{1,128}$/.test(principal)) return null;
  const now = Math.floor(nowMs / 1000);
  return { k: "session", sub: principal, org: ORG, iat: now, exp: now + SESSION_TTL_S };
}
```

и в `currentSession` — третий источник между кукой и обходом:

```ts
openSession(...) ?? trustedProxySession(req) ?? localDevSession(req)
```

Затем в `/etc/caddy/Caddyfile`, внутри блока `code.undassa.com`, в `reverse_proxy`
на 8097:

```
header_up X-Forwarded-Principal {http.auth.user.id}
```

**Принять двумя сломами, не одним.** Первый: с домена под паролем пускает.
Второй, тот, который обычно не делают: заголовок, посланный НЕ с loopback,
пускать не должен — проверить с другой машины, минуя Caddy.

### 2. Мост харнеса — первый шаг переработки

`HARNESS.md` в форке описывает шесть изменений. Первый шаг маленький и
проверяющий, а не обязывающий: **провести одну задачу myack как `run` их же
воркером**, автомат оставить снаружи.

Мешает одно: репозиториев myack и tot-ade и их волтов на этой машине нет.
Либо перенести, либо начинать с проекта, который здесь есть.

### 3. Мелочи, которые сэкономят время

- **`ssh` в профиле сломан** — `command not found: _project_ssh`. Работает
  `/usr/bin/ssh` абсолютным путём.
- **Node здесь v22.22.2 и гоняет TypeScript нативно**, но срезанием типов:
  параметрических свойств конструктора, `enum`, `namespace` и декораторов нет.
- **Секреты ядра проверяются на стойкость**, когда задан `DATABASE_URL`:
  плейсхолдеры вроде `dev-...` отвергаются. Генерировать `openssl rand -hex 32`.
- **`ADMIN_GRANTS` в окружении не сеет грант, когда есть Postgres** — посев пуст
  по построению. Выдавать строкой в `admin_grants`.
- **Форма принципала — `имя@ORG_ID`.** `adminActorFrom` склеивает их сам, поэтому
  грант на `undassa` не действует, а на `undassa@undassa` действует.
- **`DEPLOY_PROVIDER=fly` требует `FLY_DEPLOY_API_TOKEN`.** Не задавать: здесь
  есть docker.

## Слепок снесённого dsh

DeepSeek Harness удалён: служба, `dsh-web-patch.sh`, глобальный пакет, `~/.dsh`.
Ничего не уничтожено безвозвратно:

```
/opt/backup/dsh-20260902-1009/
    Caddyfile          до правки
    dsh-web.service
    dsh-web-patch.sh
    dsh-home.tgz       ~/.dsh, 22 МБ
```

## Чего я НЕ делал

- не пушил коммит с gpg-ключом в `devops/cicd` (ждёт в `main`);
- не применял патч пути Б;
- не переключал корень домена;
- не запускал ни одной настоящей сессии агента: везде подставной исполнитель.

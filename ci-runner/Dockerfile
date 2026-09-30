# Раннер CI, отрезанный от машины. Он выполняет содержимое воркфлоу как
# обычные команды, а на этой машине рядом стоят `mh-web`, общая база и чекауты
# обоих наборов. Поэтому контейнер, свой пользователь и никаких монтирований
# хоста: рабочий каталог живёт внутри, на томе.
#
# Раннер берётся официальным архивом с github.com/actions/runner — первой
# рукой, а не образом третьей стороны: эта штука исполняет код, и цепочку
# поставки ей укорачивают, а не удлиняют.
FROM ubuntu:24.04

ARG RUNNER_VERSION=2.337.0

ENV DEBIAN_FRONTEND=noninteractive
# `python3` — потому что рецепты наборов его зовут: у tot-ade `just` берёт им
# список своих пакетов (`ours`), `just red` и `just landed` — свои проверки.
# Без него `lint` падал кодом 127, а `fmt-check` молча получал пустой список
# пакетов и проверял не то, что должен: `python3` внутри `$(…)` `set -e` не
# ловит. Хостовый раннер GitHub его имел, и при переезде на свой он пропал.
# `openssh-server` и `openssh-client` — удалённая половина матрицы `NFR-10`
# (`Q-165`, форма `4а`). Решение владельца 2026-09-24: sshd поднимается ОБЫЧНЫМ
# ПРОЦЕССОМ в этом же контейнере, без root, на высоком порту, вход тем же
# пользователем `runner` — правил `sudoers` это не добавляет, вложенного движка
# контейнеров не требует. Тот же приём, что у postgres рядом: сервер ставится в
# образ, а не просится блоком `services:`.
#
# Поднимает его сама проверка `nfr_10_process_tree_on_every_target_os`: свой ключ,
# свой конфиг, свой каталог в `/tmp`. Без этих двух пакетов она красна — и красна
# честно: зелёная оттого, что механизма нет, она была бы неотличима от проверки,
# ничего не спросившей.
# `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libgtk-3-dev` — окно tot-ade
# (`tot-window`, Tauri) входит в воркспейс, и любая сборка `--workspace` на Linux
# линкует WebKitGTK (решение держателя Q-214, 2026-09-30). Ставить их шагом
# воркфлоу нельзя: `runner` получает sudo только на `pg_ctlcluster`, и так и
# должно остаться — шаг с `apt-get` валил бы все Linux-работы.
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq tar gzip xz-utils unzip sudo \
      build-essential pkg-config libssl-dev \
      postgresql postgresql-contrib \
      openssh-server openssh-client \
      libwebkit2gtk-4.1-dev libsoup-3.0-dev libgtk-3-dev \
      python3 \
 && rm -rf /var/lib/apt/lists/*

# Правила доступа переписываются ЦЕЛИКОМ, а не правятся `sed`-ом: у строки
# `host` между «all all» и методом стоит адрес, и образец, снимавший «всё после
# all all», оставлял строку без метода — postgres отказывался стартовать с
# «end-of-line before authentication method». Три строки короче любой правки.
#
# Postgres ставится В ОБРАЗ, а не поднимается блоком `services:` воркфлоу.
# `services:` просит докер ВНУТРИ раннера, а дать его можно лишь пробросив
# сокет хоста — то есть отдав раннеру весь докер машины, где стоят `mh-web` и
# общая база. Это ровно та изоляция, ради которой контейнер и заводился, и
# менять её на удобство одной работы нельзя. Локальный сервер стоит секунду
# старта и никому ничего не открывает.
RUN set -eux; \
    conf=$(dirname "$(find /etc/postgresql -name postgresql.conf | head -1)"); \
    printf '%s\n' \
      'local   all all       trust' \
      'host    all all 127.0.0.1/32 trust' \
      'host    all all ::1/128      trust' > "$conf/pg_hba.conf"; \
    echo "listen_addresses = 'localhost'" >> "$conf/postgresql.conf"; \
    echo "runner ALL=(ALL) NOPASSWD: /usr/bin/pg_ctlcluster" > /etc/sudoers.d/pg; \
    chmod 440 /etc/sudoers.d/pg

# Node и системные библиотеки Chromium — веб-проверки tot-ade (`M5-T99`: Svelte,
# `just web-test` под Playwright с настоящим движком браузера). Node берётся
# официальным архивом с nodejs.org со сверкой sha256, а не пакетом дистрибутива:
# у Ubuntu 24.04 он 18-й, а набор зафиксировал 22-й. Библиотеки ОС для Chromium
# ставит сам Playwright (`install-deps`) — от root и только при сборке образа:
# у пользователя `runner` sudo нет и не будет. Сам браузер качает
# `npx playwright install chromium` в свой кеш уже в работе (карточка `M5-T99`).
# Версии — те, что назвал исполнитель `M5-T95`/`M5-T99` 2026-09-30.
ARG NODE_VERSION=22.22.1
ARG NODE_SHA256=9a6bc82f9b491279147219f6a18add1e18424dce90d41d2a5fcd69d4924ba3aa
ARG PLAYWRIGHT_VERSION=1.63.0
RUN set -eux; \
    curl -fsSL -o /tmp/node.tar.xz \
      "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz"; \
    echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c -; \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1; \
    rm /tmp/node.tar.xz; \
    node --version; \
    apt-get update; \
    npx -y "playwright@${PLAYWRIGHT_VERSION}" install-deps chromium; \
    rm -rf /var/lib/apt/lists/* /root/.npm

# Вторая группа — затем, чтобы проверка «сохранение не теряет группу файла»
# могла упасть: она меняет группу файла на другую группу пользователя, и у
# пользователя с одной группой ей не на что менять. Без этого tot-ade
# `fr_80_save_is_atomic_and_keeps_file_properties` честно краснел на Linux —
# «сохранение группы на этой машине проверить нечем».
RUN useradd -m -s /bin/bash runner \
 && groupadd --system ci-second \
 && usermod -aG ci-second runner
WORKDIR /home/runner
RUN curl -fsSL -o runner.tar.gz \
      "https://github.com/actions/runner/releases/download/v${RUNNER_VERSION}/actions-runner-linux-x64-${RUNNER_VERSION}.tar.gz" \
 && tar xzf runner.tar.gz && rm runner.tar.gz \
 && ./bin/installdependencies.sh \
 && chown -R runner:runner /home/runner

COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

USER runner
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]

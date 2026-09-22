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
RUN apt-get update && apt-get install -y --no-install-recommends \
      ca-certificates curl git jq tar gzip unzip sudo \
      build-essential pkg-config libssl-dev \
      postgresql postgresql-contrib \
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

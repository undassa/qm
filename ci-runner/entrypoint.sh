#!/usr/bin/env bash
# Раннер на одну работу: `--ephemeral` снимает его с учёта, как только работа
# кончилась, и контейнер выходит. Состояние между работами не копится, и
# следующая начинается с чистого дерева — то же, что даёт хостовый раннер.
#
# Внутрь едет ТОЛЬКО токен регистрации: короткоживущий (час) и годный ровно на
# то, чтобы зарегистрировать раннер. Личный токен с правами на организацию сюда
# не попадает НИКОГДА — шаг воркфлоу читает окружение процесса, и любой такой
# токен утёк бы первым же `env`.
set -euo pipefail

: "${REPO:?не задан REPO, например tot-space/tot-ade}"
: "${RUNNER_TOKEN:?не задан RUNNER_TOKEN — токен регистрации чеканит хост}"

# ЧИСЛО ПОТОКОВ БЕРЁТСЯ У КВОТЫ, А НЕ У ЯДЕР ХОЗЯИНА. `--cpus` ставит квоту
# cgroup и НЕ трогает привязку, а `nproc` и `available_parallelism()` читают
# именно привязку: замерено — внутри контейнера с квотой в три ядра оба
# отвечают 16, `Cpus_allowed_list` равен `0-15`, а `cpu.max` при этом
# `300000 100000`. Cargo пускал шестнадцать сборок и шестнадцать тестов на три
# ядра-секунды, и postgres, живущий в этом же контейнере, переставал отвечать
# на подключение посреди работы. Наружу это выходило четырьмя упавшими тестами
# про базу — то есть видом дефекта правки, которой там не было.
#
# Снижение самой квоты снаружи этого НЕ лечит: перепродажа живёт внутри, и при
# любом `--cpus` контейнер планировал бы работу как на шестнадцати. Лечит
# только согласие числа потоков с квотой, и делать это надо здесь — через этот
# файл проходит каждая работа каждого раннера.
# Имена здесь латиницей: bash не берёт не-ASCII в имени переменной, и
# `threads=1` кириллицей падает «command not found» — раннер умер бы на старте.
quota=$(cut -d' ' -f1 /sys/fs/cgroup/cpu.max 2>/dev/null || echo max)
period=$(cut -d' ' -f2 /sys/fs/cgroup/cpu.max 2>/dev/null || echo 100000)
if [ "$quota" != "max" ] && [ "${period:-0}" -gt 0 ]; then
  threads=$(( quota / period ))
  if [ "$threads" -lt 1 ]; then
    threads=1
  fi
  export CARGO_BUILD_JOBS="$threads" RUST_TEST_THREADS="$threads" NEXTEST_TEST_THREADS="$threads"
  echo "квота cgroup — $threads ядер, столько же потоков сборки и тестов"
fi

# Локальная база под работы, которым она нужна: тот же адрес, что давал
# `services: postgres`, — postgres://postgres@localhost:5432/postgres.
# Без `|| true`: не поднявшаяся база должна ронять раннер здесь, а не через
# десять минут девятью тестами с «Connection refused».
sudo pg_ctlcluster "$(ls /etc/postgresql | head -1)" main start
pg_isready -h localhost -t 30

./config.sh \
  --url "https://github.com/${REPO}" \
  --token "${RUNNER_TOKEN}" \
  --name "${RUNNER_NAME:-ci-$(hostname)}" \
  --labels "${RUNNER_LABELS:-self-hosted,linux,x64}" \
  --work /home/runner/work \
  --unattended --replace --ephemeral

exec ./run.sh

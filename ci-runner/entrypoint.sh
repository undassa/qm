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

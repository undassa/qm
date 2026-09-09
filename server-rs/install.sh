#!/usr/bin/env bash
# Установка mh-web и починка путей у служб-доноров.
#
# Запускать под sudo: пишет в /etc/systemd/system. Идемпотентно — можно повторять.
set -euo pipefail

MH=/opt/src/github.com/undassa/mh

echo "1. Пути служб-доноров: plugins/ переехал в legacy-qm/, юниты об этом не знают."
echo "   Сейчас портал, брокер входа и админка живут только тем, что запущены до переезда."
for pair in "portal:mh-portal" "auth:mh-auth" "admin:mh-admin" "web-ui:mh-ui"; do
  dir=${pair%%:*}; unit=${pair##*:}
  install -d "/etc/systemd/system/$unit.service.d"
  printf '[Service]\nWorkingDirectory=%s/legacy-qm/plugins/%s\n' "$MH" "$dir" \
    > "/etc/systemd/system/$unit.service.d/10-legacy-path.conf"
  echo "   $unit → legacy-qm/plugins/$dir"
done
install -d /etc/systemd/system/mh.service.d
printf '[Service]\nWorkingDirectory=%s/legacy-qm\n' "$MH" > /etc/systemd/system/mh.service.d/10-legacy-path.conf
echo "   mh → legacy-qm"

echo "2. Новая служба mh-web: занимает 127.0.0.1:8096 вместо прежней поверхности."
install -m 0644 "$MH/server-rs/mh-web.service" /etc/systemd/system/mh-web.service
touch /var/log/mh-web.log && chown undassa:undassa /var/log/mh-web.log

echo "3. Освобождаю порт от ручного запуска, если он остался с наладки."
pkill -x mh-server || true
sleep 1

echo "4. Поднимаю."
systemctl daemon-reload
systemctl enable --now mh-web.service
sleep 1

echo
systemctl is-active mh-web.service mh-portal.service mh-auth.service | paste -sd' ' - \
  | sed 's/^/   mh-web mh-portal mh-auth: /'
echo -n "   интерфейс на 8096: "; curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:8096/next/
echo -n "   вход снаружи:      "; curl -s -o /dev/null -w '%{http_code}\n' https://code.undassa.com/

#!/usr/bin/env bash
# Самотест харнеса — шесть прогонов, а не два.
#
# Проба, живая только в сегодняшнем состоянии, не доказывает ничего и читается
# точно как рабочая: пробы ступеней 9 и 10 переоткрывали закрытую задачу ОТКРЫТОЙ
# фазы и назывались живыми, пока такая фаза была. Живой считается та, что роняет
# своё правило и при всех пройденных гейтах, и при всех красных.
set -uo pipefail

bad=0

check () {
  local door=$1 under=$2 out rc
  # БЕЗ `set -e` И С ПОЙМАННЫМ КОДОМ. `mh call` выходит единицей на отказе и
  # двойкой на обрыве, а сам отказ печатает в stdout — под `set -e` скрипт умирал
  # на подстановке, и работа падала вообще без строчки о причине. Ровно та
  # красная проверка без довода, против которой эта работа и заведена.
  out=$(mh call "$door" ${under:+under=$under} 2>&1); rc=$?
  if [ $rc -ne 0 ]; then
    printf '%-14s under=%-9s ОТКАЗ (код %s)\n' "$door" "${under:-как-есть}" "$rc"
    printf '%s\n' "$out" >&2
    bad=1
    return
  fi
  # ПУСТОЙ НАБОР — НЕ «ПРОЙДЕНО». Оба самотеста отвечают «сломанных 0» и когда
  # проверять нечего: чужой адрес, свежая база, несуществующий процесс. Шесть
  # строк нулей и выход нулём — это проверка, которая доказывает, что скрипт
  # запустился, а не что харнес цел.
  printf '%s' "$out" | python3 -c '
import json, sys
d = json.load(sys.stdin)
alive = d.get("alive", 0)
broken = d.get("broken", 1)
undeclared = d.get("undeclared", 0)
print("%d %d %d" % (alive, broken, undeclared))
for key in ("brokenItems", "brokenSteps", "undeclaredItems", "undeclaredSteps", "staleItems"):
    for row in d.get(key, []):
        print("   ", key, json.dumps(row, ensure_ascii=False), file=sys.stderr)
' > /tmp/mh-selftest.$$ 2>/tmp/mh-selftest.err.$$ || { printf '%s\n' "$out" >&2; bad=1; return; }
  read -r alive broken undeclared < /tmp/mh-selftest.$$
  rm -f /tmp/mh-selftest.$$
  printf '%-14s under=%-9s живых %-3s · сломанных %s · без пробы %s\n' \
    "$door" "${under:-как-есть}" "$alive" "$broken" "$undeclared"
  if [ "$alive" -eq 0 ]; then
    echo "   живой пробы нет ни одной: не проверено ничего, и это не «пройдено»" >&2
  fi
  if [ "$alive" -eq 0 ] || [ "$broken" != 0 ] || [ "$undeclared" != 0 ]; then
    cat /tmp/mh-selftest.err.$$ >&2
    bad=1
  fi
  rm -f /tmp/mh-selftest.err.$$
}

for under in "" green red; do
  check gate-selftest "$under"
  check step-selftest "$under"
done
exit "$bad"

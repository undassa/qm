# Приватный раннер CI

Работы `runs-on: [self-hosted, linux, x64]` исполняет одноразовый контейнер: одна
работа — один раннер, после неё он снимается с учёта и контейнер умирает. Дерево
живёт внутри контейнера, монтирований хоста нет вовсе: рядом на машине стоят
`mh-web`, общая база и чекауты наборов, а воркфлоу исполняет свои команды как
есть.

## Откуда это берётся

Истина — этот каталог. На машине файлы лежат копией в `/opt/ci-runner`, потому
что служба не должна зависеть от того, на какой ветке сейчас чекаут репозитория.
Правка идёт сюда, а потом раскладывается:

```
sudo install -o undassa -g undassa -m 0755 ci-runner/run-one.sh ci-runner/entrypoint.sh /opt/ci-runner/
sudo install -o undassa -g undassa -m 0644 ci-runner/Dockerfile /opt/ci-runner/
sudo install -m 0644 ci-runner/ci-runner@.service /etc/systemd/system/
sudo systemctl daemon-reload
docker build -t ci-runner:local /opt/ci-runner
```

Образ пересобирается при правке `Dockerfile` или `entrypoint.sh`; уже запущенный
контейнер держит прежний образ, поэтому после сборки службы перезапускаются:

```
sudo systemctl restart 'ci-runner@tot\x2dspace-tot\x2dade:2.service'
```

## Сколько их

Имя экземпляра — репозиторий с экранированной косой и, через двоеточие, номер:
раннер берёт ОДНУ работу за раз, а прогон набора состоит из шести работ.

```
sudo systemctl enable --now 'ci-runner@undassa-mh:1.service'
sudo systemctl enable --now 'ci-runner@tot\x2dspace-tot\x2dade:1.service'
```

## Чего в образе не хватало и почему это здесь записано

- `python3` — воркфлоу набора зовёт его скриптами проверки, а базовый образ его
  не несёт;
- вторая группа у пользователя `runner` (`ci-second`) — проверка сохранения
  владельца файла требует пользователя, состоящего более чем в одной группе, и
  без неё работа падала с «у пользователя нет второй группы».

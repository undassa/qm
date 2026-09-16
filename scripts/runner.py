#!/usr/bin/env python3
# Воркер пульта: кто-то должен отвечать в беседах набора и вести прогоны задач,
# иначе строка человека висит непрочитанной, а пульт показывает разговор с
# пустотой.
#
# Беседа и прогон отвечаются сессией Claude в дереве набора: ей видны и дерево,
# и двери. Имя сессии беседа и прогон помнят, поэтому вторая строка попадает в
# тот же разговор, а не в чистый лист.
#
# Останавливается прогон на двух вещах: на вопросе владельцу и на покрасневшем
# гейте. Коммит и пуш кода он не делает сам — просит подтверждение.
import json
import os
import subprocess
import sys
import time

MH = os.path.expanduser("~/.local/bin/mh")
# Ключ --allowedTools забирает ВСЕ следующие слова, поэтому список идёт одним
# словом через запятую, а сама просьба — стандартным вводом.
БЕСЕДА = "Read,Grep,Glob,Bash(mh:*),Bash(git log:*),Bash(git diff:*),Bash(git status:*),Bash(ls:*),Bash(rg:*)"
РАБОТА = "Read,Grep,Glob,Edit,Write,MultiEdit,TodoWrite,Bash"
ЗАПРЕТ = "Bash(git commit:*),Bash(git push:*)"
ПОРА = 5
СРОК = 1800

ПРАВИЛА = """Веди задачу {task} набора «{name}». Прогон {run}.

Правила прогона:
- Работай в этом дереве. Набор правь дверями `mh call`, переменная MH_PROJECT задана.
- Коммит и пуш кода сам не делай. Когда правки готовы, позови
  `mh call approval-ask runId={run} title="что коммитим" body="дифф коротко"` и остановись.
- Нужен ответ владельца — позови `mh call question-ask runId={run} title="вопрос" body="что известно"` и остановись.
- Каждый заметный шаг отмечай `mh call run-event runId={run} text="что сделано"`.
- Ничего не обходи: если правило гейта краснеет, чини причину, а не правило.

Начни с того, что прочитай задачу и её требования, и скажи в конце одним абзацем, что сделано."""


def дверь(project: str, name: str, **args: str) -> dict:
    cmd = [MH, "call", name] + [f"{k}={v}" for k, v in args.items()]
    r = subprocess.run(cmd, env={**os.environ, "MH_PROJECT": project}, capture_output=True, text=True)
    out = r.stdout.strip()
    if r.returncode != 0 or not out.startswith("{"):
        raise RuntimeError(f"{name}: {(out + r.stderr)[:400]}")
    return json.loads(out)


def длинным(project: str, name: str, ключ: str, текст: str, **args: str) -> dict:
    путь = f"/tmp/mh-runner-{os.getpid()}.txt"
    with open(путь, "w", encoding="utf-8") as f:
        f.write(текст)
    try:
        return дверь(project, name, **args, **{ключ: "@" + путь})
    finally:
        os.unlink(путь)


def спросить(набор: dict, session: str, prompt: str, tools: str, deny: str = "") -> tuple[str, str]:
    cmd = ["claude", "-p", "--output-format", "json", "--allowedTools", tools]
    if deny:
        cmd += ["--disallowedTools", deny]
    if session:
        cmd += ["--resume", session]
    try:
        r = subprocess.run(
            cmd,
            input=prompt,
            cwd=набор["repo"],
            env={**os.environ, "MH_PROJECT": набор["project"]},
            capture_output=True,
            text=True,
            timeout=СРОК,
        )
    except subprocess.TimeoutExpired:
        return f"сессия не уложилась в {СРОК} с", session
    if r.returncode != 0:
        return f"сессия не ответила: {(r.stdout + r.stderr)[-600:]}", session
    try:
        d = json.loads(r.stdout)
    except json.JSONDecodeError:
        return f"ответ не разобран: {r.stdout[-600:]}", session
    return str(d.get("result", "")).strip() or "пустой ответ", str(d.get("session_id", session))


def беседы(набор: dict) -> None:
    project = набор["project"]
    for б in дверь(project, "chat").get("threads", []):
        if not б.get("waiting"):
            continue
        thread = б["thread"]
        коробка = дверь(project, "chat-inbox", thread=thread)
        строки = [s["text"] for s in коробка.get("said", [])]
        if not строки:
            continue
        print(f"{набор['name']} · беседа {thread}: {len(строки)} строк", flush=True)
        подсказка = (
            f"Ты отвечаешь в беседе пульта харнеса про набор «{набор['name']}». Отвечай по-русски, коротко и по делу. "
            f"Двери набора: `mh call <дверь> k=v`. Беседа — это размышление: файлы не правь и не коммить, "
            f"а скажи, что нужно сделать.\n\n" + "\n\n".join(строки)
        )
        ответ, session = спросить(набор, коробка.get("sessionId", ""), подсказка, БЕСЕДА)
        длинным(project, "chat-say", "text", ответ, thread=thread, side="agent")
        if session:
            дверь(project, "chat-inbox", thread=thread, sessionId=session)


def красных(project: str) -> int:
    # Счёт непройденных ПУНКТОВ, а не гейтов: гейт краснеет от одного пункта, и
    # по нему не видно, стало хуже или лучше.
    гейт = дверь(project, "gate")
    return sum(
        1
        for g in гейт.get("gates", [])
        for i in g.get("items", [])
        if i.get("computed") not in ("passed", None)
    )


def прогоны(набор: dict) -> None:
    project = набор["project"]
    for run in дверь(project, "runs", limit="10").get("runs", []):
        if run["state"] not in ("running", "waiting"):
            continue
        решения = дверь(project, "ask-inbox", runId=run["runId"]).get("decided", [])
        if run["state"] == "waiting" and not решения:
            continue
        if run["state"] == "running" and run.get("sessionId") and not решения:
            continue
        одобрено = any(d["kind"] == "approval" and d["state"] == "approved" for d in решения)
        if решения:
            prompt = "Решение владельца:\n" + "\n".join(
                f"- {d['title']} → {d['state']}: {d['why']}" for d in решения
            ) + "\n\nПродолжай прогон с учётом этого."
        else:
            prompt = ПРАВИЛА.format(task=run["task"], name=набор["name"], run=run["runId"])
        print(f"{набор['name']} · прогон {run['task']} ({run['state']})", flush=True)
        было = красных(project)
        ответ, session = спросить(
            набор, run.get("sessionId", ""), prompt, РАБОТА, "" if одобрено else ЗАПРЕТ
        )
        длинным(project, "run-event", "text", ответ[:4000], runId=run["runId"], kind="итог")
        открытые = [
            a for a in дверь(project, "asks", state="open").get("asks", []) if a["runId"] == run["runId"]
        ]
        стало = красных(project)
        if открытые:
            состояние, чем = "waiting", f"ждёт решения владельца: {открытые[0]['title']}"
        elif стало > было:
            состояние, чем = "waiting", f"гейт покраснел: было {было}, стало {стало}"
            длинным(project, "question-ask", "body", ответ[:2000],
                    runId=run["runId"], title=f"{run['task']}: гейт покраснел ({было} → {стало})")
        else:
            состояние, чем = "done", ответ[:300]
        дверь(project, "run-state", runId=run["runId"], state=состояние, note=чем, session=session)
        return  # один прогон за круг: беседа не должна ждать конца задачи


def круг(наборы: list[dict]) -> None:
    for набор in наборы:
        try:
            беседы(набор)
            прогоны(набор)
        except RuntimeError as e:
            print(f"{набор['name']}: {e}", flush=True)


def main() -> None:
    путь = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "runner.json")
    наборы = json.load(open(путь, encoding="utf-8"))["projects"]
    for набор in наборы:
        if not os.path.isdir(набор["repo"]):
            sys.exit(f"нет дерева {набор['repo']} — воркеру нечего открыть")
    print(f"воркер пульта: наборов {len(наборы)}, круг {ПОРА} с", flush=True)
    while True:
        try:
            круг(наборы)
        except Exception as e:  # круг не должен уносить воркер: следующий круг важнее
            print(f"круг сорвался: {e}", flush=True)
        time.sleep(ПОРА)


if __name__ == "__main__":
    main()

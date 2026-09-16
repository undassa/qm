#!/usr/bin/env python3
# Воркер пульта: кто-то должен отвечать в беседах набора, иначе строка человека
# висит непрочитанной, а пульт показывает разговор с пустотой.
#
# Беседа отвечается сессией Claude в папке репозитория набора: ей видны и дерево,
# и двери набора. Имя сессии беседа помнит, поэтому вторая строка попадает в тот
# же разговор, а не в чистый лист.
import json
import os
import subprocess
import sys
import time

MH = os.path.expanduser("~/.local/bin/mh")
# Ключ --allowedTools забирает ВСЕ следующие слова, поэтому список идёт одним
# словом через запятую, а сама просьба — стандартным вводом.
ВЕДРО = "Read,Grep,Glob,Bash(mh:*),Bash(git log:*),Bash(git diff:*),Bash(git status:*),Bash(ls:*),Bash(rg:*)"
ПОРА = 5


def дверь(project: str, name: str, **args: str) -> dict:
    cmd = [MH, "call", name] + [f"{k}={v}" for k, v in args.items()]
    r = subprocess.run(cmd, env={**os.environ, "MH_PROJECT": project}, capture_output=True, text=True)
    out = r.stdout.strip()
    if r.returncode != 0 or not out.startswith("{"):
        raise RuntimeError(f"{name}: {(out + r.stderr)[:400]}")
    return json.loads(out)


def сказать(project: str, thread: str, text: str) -> None:
    путь = f"/tmp/mh-runner-{thread}.txt"
    with open(путь, "w", encoding="utf-8") as f:
        f.write(text)
    дверь(project, "chat-say", thread=thread, side="agent", text="@" + путь)
    os.unlink(путь)


def спросить(набор: dict, session: str, prompt: str) -> tuple[str, str]:
    подсказка = (
        f"Ты отвечаешь в беседе пульта харнеса про набор «{набор['name']}». Отвечай по-русски, коротко и по делу. "
        f"Двери набора: `mh call <дверь> k=v`, переменная MH_PROJECT уже задана. "
        f"Беседа — это размышление: файлы не правь и не коммить, а скажи, что нужно сделать."
    )
    cmd = [
        "claude", "-p", "--output-format", "json",
        "--append-system-prompt", подсказка,
        "--allowedTools", ВЕДРО,
    ]
    if session:
        cmd += ["--resume", session]
    r = subprocess.run(
        cmd,
        input=prompt,
        cwd=набор["repo"],
        env={**os.environ, "MH_PROJECT": набор["project"]},
        capture_output=True,
        text=True,
        timeout=900,
    )
    if r.returncode != 0:
        return f"сессия не ответила: {(r.stdout + r.stderr)[-600:]}", session
    try:
        d = json.loads(r.stdout)
    except json.JSONDecodeError:
        return f"ответ не разобран: {r.stdout[-600:]}", session
    return str(d.get("result", "")).strip() or "пустой ответ", str(d.get("session_id", session))


def круг(наборы: list[dict]) -> None:
    for набор in наборы:
        project = набор["project"]
        try:
            беседы = дверь(project, "chat").get("threads", [])
        except RuntimeError as e:
            print(f"{набор['name']}: {e}", flush=True)
            continue
        for б in беседы:
            if not б.get("waiting"):
                continue
            thread = б["thread"]
            коробка = дверь(project, "chat-inbox", thread=thread)
            строки = [s["text"] for s in коробка.get("said", [])]
            if not строки:
                continue
            print(f"{набор['name']} · {thread}: {len(строки)} строк", flush=True)
            ответ, session = спросить(набор, коробка.get("sessionId", ""), "\n\n".join(строки))
            сказать(project, thread, ответ)
            if session:
                дверь(project, "chat-inbox", thread=thread, sessionId=session)


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

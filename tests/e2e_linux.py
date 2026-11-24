"""E2E на Linux в CI: Xvfb + openbox + xterm.

Проверяет: список окон, выбор окна (через trusted_apps, без диалога), снимок только окна,
клик, ввод текста и Enter (команда реально выполняется в xterm), блок номера карты,
стоп-клавишу Ctrl+Alt+Q и запись журнала. Только стандартная библиотека Python.
"""
import base64
import json
import os
import pathlib
import subprocess
import sys
import time

BIN = sys.argv[1]
CFG_DIR = pathlib.Path(os.environ.get("XDG_CONFIG_HOME", pathlib.Path.home() / ".config")) / "WorkDeskPilot"
LOG_DIR = pathlib.Path(os.environ.get("XDG_DATA_HOME", pathlib.Path.home() / ".local/share")) / "WorkDeskPilot" / "logs"
OUT = pathlib.Path("/tmp/wdpilot-e2e.txt")

fails = 0


def check(ok, what):
    global fails
    print(("OK   " if ok else "FAIL ") + what, flush=True)
    if not ok:
        fails += 1


class Server:
    def __init__(self):
        self.p = subprocess.Popen([BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.n = 0
        r = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                    "clientInfo": {"name": "e2e", "version": "0"}})
        assert r["result"]["serverInfo"]["name"] == "wdpilot"
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")

    def rpc(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, name, args=None):
        res = self.rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        text = " ".join(c.get("text", "") for c in res["content"] if c["type"] == "text")
        print(f"  {name} {json.dumps(args or {}, ensure_ascii=False)} -> {'ERR ' if res['isError'] else ''}{text[:160]}", flush=True)
        return res

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


s = Server()
target, wins = None, []
for _ in range(15):  # xterm под Xvfb иногда стартует дольше пары секунд
    res = s.call("list_windows")
    if res["isError"]:  # оконный менеджер ещё не поднялся (_NET_CLIENT_LIST_STACKING)
        time.sleep(1)
        continue
    wins = json.loads(res["content"][0]["text"])
    target = next((w for w in wins if "e2e-term" in w["title"] or "xterm" in w["app"].lower()), None)
    if target:
        break
    time.sleep(1)
check(target is not None, "xterm найден в list_windows")
if target is None:
    print(json.dumps(wins, ensure_ascii=False, indent=1))
    sys.exit(1)

res = s.call("screenshot")
check(res["isError"], "screenshot без выбранного окна запрещён")
s.close()

# Доверенное приложение — чтобы в CI не было диалога согласия.
CFG_DIR.mkdir(parents=True, exist_ok=True)
(CFG_DIR / "config.json").write_text(json.dumps({"trusted_apps": [target["app"]]}), encoding="utf-8")

s = Server()
check(not s.call("select_window", {"window_id": target["window_id"]})["isError"], "select_window")

res = s.call("screenshot")
img = next((c for c in res["content"] if c["type"] == "image"), None)
png = base64.b64decode(img["data"]) if img else b""
check(png[:8] == b"\x89PNG\r\n\x1a\n", "screenshot отдаёт PNG")
w = int.from_bytes(png[16:20], "big") if png else 0
h = int.from_bytes(png[20:24], "big") if png else 0
check(abs(w - target["width"]) <= 4 and abs(h - target["height"]) <= 4, f"снимок только окна ({w}x{h})")

check(not s.call("left_click", {"coordinate": [w // 2, h // 2]})["isError"], "left_click внутри окна")
check(s.call("left_click", {"coordinate": [w + 50, 10]})["isError"], "клик вне снимка отклонён")
OUT.unlink(missing_ok=True)
check(not s.call("type", {"text": f"echo wdpilot-ok > {OUT}"})["isError"], "type")
check(not s.call("key", {"text": "Return"})["isError"], "key Return")
s.call("wait", {"duration": 1})
check(OUT.exists() and OUT.read_text().strip() == "wdpilot-ok", "команда реально выполнена в xterm")

check(s.call("type", {"text": "4111 1111 1111 1111"})["isError"], "номер карты не вводится")
check(s.call("key", {"text": "ctrl+nonsense"})["isError"], "неверная клавиша — понятная ошибка")

subprocess.run(["xdotool", "keydown", "ctrl+alt+q"], check=False)
time.sleep(0.3)
subprocess.run(["xdotool", "keyup", "ctrl+alt+q"], check=False)
time.sleep(0.5)
res = s.call("screenshot")
txt = " ".join(c.get("text", "") for c in res["content"] if c["type"] == "text")
check(res["isError"] and "Остановлено" in txt, "Ctrl+Alt+Q останавливает и отзывает доступ")
check(s.call("left_click", {"coordinate": [10, 10]})["isError"], "после стопа действия запрещены")
s.close()

logs = list(LOG_DIR.glob("*.jsonl"))
lines = [json.loads(l) for f in logs for l in f.read_text(encoding="utf-8").splitlines() if l.strip()]
check(any(l["action"] == "kill_switch" for l in lines), "стоп записан в журнал")
typed = [l for l in lines if l["action"] == "type"]
check(bool(typed) and all("text" not in l["args"] for l in typed), "введённый текст не попадает в журнал")

print(f"\nИтог: {'ПРОЙДЕНО' if fails == 0 else f'ошибок {fails}'}")
sys.exit(1 if fails else 0)

"""通过 DAP 协议直接驱动 probe-rs dap-server，验证 launch.json 的配置能否真正进入调试。"""

import json
import os
import queue
import subprocess
import threading
import time

ROOT = os.path.abspath(os.getcwd())
CHIP_YAML = os.path.join(ROOT, "tools", "CW32L0_Series.yaml")
PROGRAM = os.path.join(
    ROOT, "target", "thumbv6m-none-eabi", "release", "examples",
    os.environ.get("DAP_PROG", "blink")
)

proc = subprocess.Popen(
    ["probe-rs", "dap-server"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)
msgs: "queue.Queue[dict]" = queue.Queue()


def reader():
    f = proc.stdout
    while True:
        length = None
        while True:
            line = f.readline()
            if not line:
                return
            line = line.strip()
            if not line:
                break
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":", 1)[1])
        if length is None:
            return
        body = f.read(length)
        if not body:
            return
        try:
            msgs.put(json.loads(body))
        except Exception as exc:  # noqa: BLE001
            msgs.put({"__parse_error__": str(exc)})


threading.Thread(target=reader, daemon=True).start()


def send(obj):
    body = json.dumps(obj).encode()
    proc.stdin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    proc.stdin.flush()


def wait_for(pred, timeout=45.0):
    deadline = time.time() + timeout
    seen = []
    while time.time() < deadline:
        try:
            m = msgs.get(timeout=1.0)
        except queue.Empty:
            continue
        seen.append(m)
        if pred(m):
            return m, seen
    return None, seen


seq = 0


def req(command, arguments=None):
    global seq
    seq += 1
    msg = {"seq": seq, "type": "request", "command": command}
    if arguments is not None:
        msg["arguments"] = arguments
    return msg


send(req("initialize", {
    "clientID": "dap-checklist",
    "adapterID": "probe-rs-debug",
    "linesStartAt1": True,
    "columnsStartAt1": True,
    "pathFormat": "path",
    "supportsVariableType": True,
}))
resp, seen = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "initialize")
print("initialize ->", "ok" if resp and resp.get("success") else ("失败: %s" % (resp or seen[-1:])))
caps = (resp or {}).get("body", {})
print("  supportsConfigurationDoneRequest =", caps.get("supportsConfigurationDoneRequest"))

launch_args = {
    "type": "probe-rs-debug",
    "request": "launch",
    "chip": "CW32L012x8",
    "chipDescriptionPath": CHIP_YAML,
    "probe": "0483:3748",
    "wireProtocol": "Swd",
    "speed": 4000,
    "flashingConfig": {"flashingEnabled": True, "haltAfterReset": True},
    "coreConfigs": [{"coreIndex": 0, "programBinary": PROGRAM}],
    "cwd": ROOT,
}
if os.environ.get("DAP_NO_YAML"):
    launch_args.pop("chipDescriptionPath")
if os.environ.get("DAP_ATTACH"):
    launch_args["request"] = "attach"
    launch_args.pop("flashingConfig", None)
    launch_args["coreConfigs"] = [{"coreIndex": 0}]
send(req("launch", launch_args))
resp, seen = wait_for(lambda m: m.get("type") == "response" and m.get("command") in ("launch", "attach"), timeout=60)
for m in seen:
    t = m.get("type")
    if t == "event":
        print("  [event]", m.get("event"), str(m.get("body"))[:220])
    elif t == "response":
        print("  [resp]", m.get("command"), m.get("success"), str(m.get("message") or "")[:120])
    elif t == "request":
        print("  [req]", m.get("command"))
if resp is None:
    print("launch -> 超时，收到事件：", [m.get("event") or m.get("command") for m in seen][-6:])
    print("stderr:", proc.stderr.read(2000).decode("utf-8", "ignore")[-800:])
else:
    print("launch ->", "success" if resp.get("success") else "失败: %s" % resp.get("message"))

send(req("configurationDone", {}))
if os.environ.get("DAP_RACE"):
    send(req("next", {"threadId": 0}))
    r, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "next", timeout=20)
    print("race next ->", (r or {}).get("success"), "|", ((r or {}).get("message") or "").replace(chr(10), " ")[:200])
    proc.kill()
    raise SystemExit
stopped, seen = wait_for(lambda m: m.get("type") == "event" and m.get("event") == "stopped", timeout=40)
if stopped:
    print("stopped ->", json.dumps(stopped.get("body", {}), ensure_ascii=False)[:400])
    tids = []
    send(req("threads", {}))
    th, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "threads", timeout=15)
    tids = [t["id"] for t in (th or {}).get("body", {}).get("threads", [])]
    print("threads ->", tids)
    if tids:
        send(req("stackTrace", {"threadId": tids[0]}))
        st, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "stackTrace", timeout=20)
        for f in (st or {}).get("body", {}).get("stackFrames", [])[:4]:
            print("   frame:", f.get("name"), "| line", f.get("line"), "| pc", f.get("instructionPointerReference"))
else:
    print("stopped -> 没等到")

if tids:
    send(req("next", {"threadId": tids[0]}))
    r, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "next", timeout=25)
    print("next ->", (r or {}).get("success"), "|", ((r or {}).get("message") or "")[:160])
    send(req("continue", {"threadId": tids[0]}))
    r2, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "continue", timeout=25)
    print("continue ->", (r2 or {}).get("success"), "|", ((r2 or {}).get("message") or "")[:160])

if os.environ.get("DAP_PAUSE"):
    send(req("continue", {"threadId": 0}))
    wait_for(lambda m: m.get("type") == "response" and m.get("command") == "continue", timeout=20)
    time.sleep(3)
    send(req("pause", {"threadId": 0}))
    pr, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "pause", timeout=20)
    print("pause ->", (pr or {}).get("success"), "|", ((pr or {}).get("message") or "")[:120])
    st2, _ = wait_for(lambda m: m.get("type") == "event" and m.get("event") == "stopped", timeout=10)
    print("pause 后 stopped ->", (st2 or {}).get("body", {}).get("reason"))
    send(req("next", {"threadId": 0}))
    n2, _ = wait_for(lambda m: m.get("type") == "response" and m.get("command") == "next", timeout=20)
    print("pause 后 next ->", (n2 or {}).get("success"), "|", ((n2 or {}).get("message") or "").replace(chr(10)," ")[:120])

send(req("disconnect", {"terminateDebuggee": True}))
time.sleep(1.0)
proc.kill()

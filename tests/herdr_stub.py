#!/usr/bin/env python3
"""A scripted herdr for tests/manager.sh: herdr_stub.py SOCK LOG [AGENTS_JSON].

One request per connection, NDJSON, as the real server does. It keeps a small live
model so the composite verbs see consistent answers: agent.start adds an agent
named as asked, pane.close removes the agents in that pane, agent.rename renames.
Every request but ping is appended to LOG as {"method", "params"}. AGENTS_JSON is a
file holding the agents live at the start.
"""
import json, os, socket, sys, time

sock_path, log_path = sys.argv[1], sys.argv[2]
agents = json.load(open(sys.argv[3])) if len(sys.argv) > 3 else []
# STUB_BUSY=N: the first N agent.start calls answer agent_pane_busy, as herdr does
# while a fresh pane's shell is still starting.
busy = [int(os.environ.get("STUB_BUSY", "0"))]


# STUB_EVENTS="MS:agent:status,..." scripts what events.subscribe streams after its ack:
# at MS since the ack, one pane.agent_status_changed for that agent. STUB_EVENTS_CLOSE=1
# then hangs up; otherwise the stream stays open until the client closes it. The stub is
# serial, so a subscription holds it until then: STUB_EVENTS_LOG, when set, gets
# "subscribed" and "closed by client" lines proving the client let go.
events = [e.split(":") for e in os.environ.get("STUB_EVENTS", "").split(",") if e]
events_log = os.environ.get("STUB_EVENTS_LOG")


def note(msg):
    if events_log:
        with open(events_log, "a") as f:
            f.write(msg + "\n")


# STUB_HANG=METHOD: that method is accepted and never answered; the stub waits for the
# client to give up and hang up (so it is serial-safe) before serving the next request.
hang = os.environ.get("STUB_HANG")


# STUB_SEAT puts a started agent behind a screen, blocked+launch_pending:
#   trust        a folder-trust dialog naming the tab's cwd; agent.send_keys clears it
#   trust-other  the same dialog naming a different folder
#   trust-blank  the dialog with no folder on it
#   stuck        a screen nothing answers
# STUB_SEAT_ONLY=SUFFIX limits that to agents whose name ends with SUFFIX.
# The dialog text is this code's fixture of what claude shows; the layout is claude's,
# the fixture only pins how the manager handles it.
seat = os.environ.get("STUB_SEAT", "")
only = os.environ.get("STUB_SEAT_ONLY", "")
cwd = ["/repo"]
# STUB_PROMPT=none: pane.read of a settled pane shows no input box.
prompt_screen = "welcome\nauto mode on\n" if os.environ.get("STUB_PROMPT") == "none" else "welcome\n> \n"


class Busy(Exception):
    pass

for p in (sock_path,):
    if os.path.exists(p):
        os.unlink(p)
srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
srv.bind(sock_path)
srv.listen(8)


def agent(name, kind, pane, status="idle"):
    behind = bool(seat) and name.endswith(only)
    if behind:
        status = "blocked"
    return {"launch_pending": behind, "name": name, "agent": kind, "agent_status": status, "workspace_id": "s",
            "tab_id": "s:t1", "pane_id": pane, "cwd": "/repo", "focused": False}


def handle(method, p):
    if method == "ping":
        return {"type": "pong", "version": "0.9.0", "protocol": 22}
    if method == "agent.list":
        return {"type": "agent_list", "agents": agents}
    if method == "tab.create":
        cwd[0] = p.get("cwd", "/repo")
        return {"type": "tab_created", "tab": {"tab_id": "s:t1"}, "root_pane": {"pane_id": "s:p1"}}
    if method == "pane.split":
        return {"type": "pane_info", "pane": {"pane_id": "s:p2"}}
    if method == "agent.start":
        if busy[0] > 0:
            busy[0] -= 1
            raise Busy()
        agents.append(agent(p["name"], p["kind"], p["pane_id"]))
    if method == "pane.close":
        agents[:] = [a for a in agents if a["pane_id"] != p["pane_id"]]
    if method == "agent.rename":
        for a in agents:
            if a["name"] == p["target"] or a["pane_id"] == p["target"]:
                a["name"] = p["name"]
    if method == "agent.send_keys":
        for a in agents:
            if a.get("launch_pending") and seat == "trust":
                a["agent_status"], a["launch_pending"] = "idle", False
    if method == "agent.wait":
        return {"type": "agent_info", "agent": {"agent_status": "idle"}}
    if method == "pane.read":
        waiting = [a for a in agents if a["pane_id"] == p["pane_id"] and a.get("launch_pending")]
        if waiting and seat.startswith("trust"):
            folder = {"trust": cwd[0], "trust-other": "/some/other/dir", "trust-blank": ""}[seat]
            text = "Do you trust this folder?\n%s\n> 1. No, exit\n  2. Yes, I trust this folder\n" % folder
        elif waiting:
            text = "Pick a theme to continue\n"
        else:
            text = prompt_screen
        return {"type": "pane_read", "read": {"text": text}}
    return {"type": "ok"}


while True:
    conn, _ = srv.accept()
    line = conn.makefile("rb").readline()
    if line:
        req = json.loads(line)
        if req["method"] != "ping":
            with open(log_path, "a") as f:
                f.write(json.dumps({"method": req["method"], "params": req["params"]}) + "\n")
        if req["method"] == hang:
            conn.settimeout(30)
            try:
                while conn.recv(4096):
                    pass
            except OSError:
                pass
            conn.close()
            continue
        if req["method"] == "events.subscribe":
            conn.sendall((json.dumps({"id": req["id"], "result": {"type": "subscription_started"}}) + "\n").encode())
            note("subscribed " + json.dumps(req["params"], sort_keys=True))
            t0 = time.time()
            for ms, name, status in events:
                time.sleep(max(0, int(ms) / 1000 - (time.time() - t0)))
                pane = next(a["pane_id"] for a in agents if a["name"] == name)
                ev = {"event": "pane.agent_status_changed",
                      "data": {"type": "pane_agent_status_changed", "pane_id": pane,
                               "workspace_id": "s", "agent_status": status}}
                conn.sendall((json.dumps(ev) + "\n").encode())
            if os.environ.get("STUB_EVENTS_CLOSE") != "1":
                conn.settimeout(30)
                try:
                    while conn.recv(4096):
                        pass
                except OSError:
                    pass
                note("closed by client")
            conn.close()
            continue
        try:
            reply = {"id": req["id"], "result": handle(req["method"], req["params"])}
        except Busy:
            reply = {"id": req["id"], "error": {"code": "agent_pane_busy",
                     "message": "agent target pane %s is not an available shell" % req["params"]["pane_id"]}}
        conn.sendall((json.dumps(reply) + "\n").encode())
    conn.close()

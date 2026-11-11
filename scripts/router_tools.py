"""Read the MCP tool router from a disposable daemon board."""

import http.client
import json
import os
from pathlib import Path
import subprocess
import tempfile


def tool_names(binary):
    with tempfile.TemporaryDirectory(prefix="agentboard-router-") as scratch:
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("AGENTBOARD_")}
        proc = subprocess.Popen(
            [str(binary), "--db", str(Path(scratch) / "board"), "daemon", "127.0.0.1:0"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env,
        )
        try:
            line = proc.stdout.readline()
            if "listening on http://" not in line:
                raise RuntimeError(f"daemon did not start: {line!r} {proc.stderr.read()}")
            host, port = line.split("http://", 1)[1].split()[0].rsplit(":", 1)
            conn = http.client.HTTPConnection(host, int(port), timeout=10)
            seq = 0

            def rpc(method, params=None, notification=False):
                nonlocal seq
                message = {"jsonrpc": "2.0", "method": method}
                if params is not None:
                    message["params"] = params
                if not notification:
                    seq += 1
                    message["id"] = seq
                conn.request("POST", "/mcp/doc", json.dumps(message), {
                    "Content-Type": "application/json",
                    "Accept": "application/json, text/event-stream",
                })
                response = conn.getresponse()
                body = response.read().decode()
                if notification:
                    return None
                if response.status != 200:
                    raise RuntimeError(f"daemon answered {response.status}: {body}")
                return json.loads(body)

            rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "router-tools", "version": "1"}})
            rpc("notifications/initialized", {}, notification=True)
            result = rpc("tools/list", {})
            conn.close()
            return {item["name"] for item in result["result"]["tools"]}
        finally:
            proc.kill()
            proc.wait()
            proc.stdout.close()
            proc.stderr.close()

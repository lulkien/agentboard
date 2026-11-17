#!/usr/bin/env bash
set -euo pipefail
bin=$1
tree=$2
db=$(mktemp -d /tmp/agentboard/scratch/bench-accept.XXXXXX)
export AGENTBOARD_DB="$db/board" AGENTBOARD_ACTOR=alice AGENTBOARD_MAP_SIZE_MB=8
long=$(python3 -c 'print("x" * 30000)')
"$bin" brief >"$db/brief-before"
[[ $(wc -l <"$db/brief-before") -eq 2 ]] || { echo "empty brief did not gain exactly one usage line" >&2; exit 1; }
"$bin" daemon 127.0.0.1:0 --db "$AGENTBOARD_DB" >"$db/daemon.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
python3 - "$db/daemon.log" "$db/health-before.json" <<'PY'
import json, re, sys, time, urllib.request
path,out=sys.argv[1:]
for _ in range(100):
    text=open(path).read()
    m=re.search(r"127\.0\.0\.1:(\d+)", text)
    if m:
        url=f"http://127.0.0.1:{m.group(1)}/api/health"
        try:
            with urllib.request.urlopen(url, timeout=.2) as r: data=json.load(r)
            open(out,"w").write(json.dumps(data))
            break
        except Exception: pass
    time.sleep(.05)
else: raise SystemExit("daemon did not expose /api/health")
assert all(key in data for key in ("board", "instance", "cursor")), data
PY
for n in $(seq 1 20); do "$bin" add --description "$long" "map growth task $n" >/dev/null; done
"$bin" brief >"$db/brief-after"
python3 - "$db/brief-before" "$db/brief-after" <<'PY'
import re, sys
def usage(path):
    lines = [line for line in open(path) if re.search(r"map", line, re.I)]
    assert len(lines) == 1, f"expected one map usage line, got {lines!r}"
    match = re.search(r"\((\d+) of \d+ bytes\)", lines[0])
    assert match, f"map usage line has no used-byte count: {lines[0]!r}"
    return int(match.group(1))
assert usage(sys.argv[1]) > 0
assert usage(sys.argv[2]) > usage(sys.argv[1]), "map usage did not rise"
PY
python3 - "$db/daemon.log" "$db/health-before.json" "$db/health-after.json" "$db/brief-before" "$db/brief-after" <<'PY'
import json, re, sys, time, urllib.request
log,before_path,after_path,brief_before,brief_after=sys.argv[1:]
m=re.search(r"127\.0\.0\.1:(\d+)",open(log).read())
assert m, "daemon address missing"
url=f"http://127.0.0.1:{m.group(1)}/api/health"
for _ in range(100):
    try:
        with urllib.request.urlopen(url, timeout=.2) as r: after=json.load(r)
        break
    except Exception: time.sleep(.05)
else: raise SystemExit("health request failed")
before=json.load(open(before_path))
def numeric_map(value):
    candidates=[v for v in value.values() if isinstance(v,dict) and any(isinstance(n,int) and not isinstance(n,bool) for n in v.values())]
    assert len(candidates)==1, value
    return candidates[0]
old,new=numeric_map(before),numeric_map(after)
assert set(("board","instance","cursor")) <= set(after), after
keys=[k for k,v in new.items() if isinstance(v,int) and not isinstance(v,bool) and k in old and isinstance(old[k],int) and v>old[k]]
assert keys, (old,new)
def brief_used(path):
    line=next(line for line in open(path) if re.search(r"map",line,re.I))
    return int(re.search(r"\((\d+) of \d+ bytes\)",line).group(1))
assert any(old[k] == brief_used(brief_before) for k in old if isinstance(old[k],int)), (old,brief_used(brief_before))
assert any(new[k] == brief_used(brief_after) for k in new if isinstance(new[k],int)), (new,brief_used(brief_after))
PY

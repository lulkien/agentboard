#!/bin/sh
# MCP entry for an opencode seat. opencode keeps one global config for every
# session, so the actor cannot live there: the seat names itself in its pane
# environment (AGENTBOARD_SEAT, set with `herdr tab create --env`), and this
# wrapper is what turns that into AGENTBOARD_ACTOR for the server.
# Without AGENTBOARD_SEAT it falls back to the name the old config carried.
actor=${AGENTBOARD_SEAT:-opencode}
bin=$(command -v agentboard 2>/dev/null || true)
[ -n "$bin" ] || bin=$HOME/.cargo/bin/agentboard
exec env AGENTBOARD_ACTOR="$actor" "$bin"

#!/bin/sh
# Fake agent for axelstra tests (Unix twin of fake-agent.ps1). Behaviour is keyed on the task id.
# $1 = marker file (used by the flaky task)
id="$AXELSTRA_TASK"
if [ "$id" = hang ]; then sleep 600; fi
if [ "$id" = flaky ] && [ ! -e "$1" ]; then : > "$1"; exit 1; fi
sleep 0.3
echo "work by $id" > "$id.txt"
if [ "$id" = conflict ]; then echo "from conflict" > shared.txt; fi
if [ "$id" = a ]; then echo "from a" > shared.txt; fi
mkdir -p .axelstra
printf '{"status":"done","summary":"did %s"}' "$id" > .axelstra/result.json

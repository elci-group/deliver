#!/bin/sh
# Example deliver plugin (protocol v1).
#
# The runner writes one JSON request line to stdin:
#   {"protocol": 1, "name": "<check name>", "base": "<absolute path>"}
# and expects one JSON response object on stdout:
#   {"pass": true|false, "message": "..."}
#
# This example reads the request, then checks that every *.md file directly
# under <base>/docs (if the directory exists) is non-empty. It deliberately
# avoids depending on a JSON tool: the request is inspected with grep only,
# and the response is printed literally.

read request || exit 1

base=$(printf '%s' "$request" | sed 's/.*"base":"\([^"]*\)".*/\1/')

if [ ! -d "$base/docs" ]; then
    echo '{"pass": true, "message": "no docs/ directory, nothing to check"}'
    exit 0
fi

empty=""
for file in "$base"/docs/*.md; do
    [ -e "$file" ] || continue
    if [ ! -s "$file" ]; then
        empty="$empty $(basename "$file")"
    fi
done

if [ -n "$empty" ]; then
    printf '{"pass": false, "message": "empty docs files:%s"}\n' "$empty"
    exit 0
fi

echo '{"pass": true, "message": "all docs/*.md files are non-empty"}'

#!/usr/bin/env bash
# Fails if any tracked (or staged) file looks like data from a game disc: disc images, executables,
# EA BIG archives, RenderWare 4 resources, or anything fg_import writes. See CLAUDE.md rule 1.
#   tools/check_no_game_data.sh
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

bad=0
names='\.(iso|xex|big|rx2|loc|abin|vp6|mus|mpf|mxb|bh|snr|sns|fgpak)$|^imported/'
hits=$(git ls-files --cached | grep -Ei "$names" || true)
if [ -n "$hits" ]; then
    echo "Disc-data file names are tracked:" >&2
    echo "$hits" | sed 's/^/  /' >&2
    bad=1
fi

# Content check on the first bytes: EA BIG v3 ("EB\0\3"), RW4 ("\x89RW4"), XEX2, XDVDFS volume, fg_import packs.
while IFS= read -r -d '' f; do
    [ -f "$f" ] || continue
    head=$(head -c 4 "$f" | od -An -tx1 | tr -d ' \n')
    case "$head" in
        45420003|89525734|58455832|46475041) echo "Disc-data content in $f (magic $head)" >&2; bad=1 ;;
    esac
    if [ "$(wc -c <"$f")" -gt 65564 ] && [ "$(tail -c +65537 "$f" | head -c 20)" = "MICROSOFT*XBOX*MEDIA" ]; then
        echo "Xbox disc image content in $f" >&2
        bad=1
    fi
done < <(git ls-files --cached -z)

if [ "$bad" -ne 0 ]; then
    echo "Game data must never be committed. Remove it with: git rm --cached <file>" >&2
    exit 1
fi
echo "OK: no game data tracked"

#!/usr/bin/env bash
set -euo pipefail

src="${1:-/etc/nixos}"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cp -a "$src"/. "$tmpdir"/

if ! nix flake update --flake "$tmpdir" >/dev/null 2>&1; then
  jq -nc \
    --arg text "!" \
    --arg tooltip "Erreur pendant nix flake update" \
    --arg class "error" \
    '{text:$text, tooltip:$tooltip, class:$class}'
  exit 0
fi

old="$(jq -c '.nodes | to_entries | map({name: .key, rev: .value.locked.rev, date: .value.locked.lastModified})' "$src/flake.lock")"
new="$(jq -c '.nodes | to_entries | map({name: .key, rev: .value.locked.rev, date: .value.locked.lastModified})' "$tmpdir/flake.lock")"

if [ "$old" = "$new" ]; then
  jq -nc \
    --arg text "" \
    --arg tooltip "Tout est à jour" \
    --arg class "uptodate" \
    '{text:$text, tooltip:$tooltip, class:$class}'
  exit 0
fi

changes="$(
  jq -n --argjson old "$old" --argjson new "$new" '
    [ $new[] as $n
      | ($old[] | select(.name == $n.name)) as $o
      | select($o.rev != $n.rev or $o.date != $n.date)
      | {
          name: $n.name,
          old_date: ($o.date | strftime("%Y-%m-%d")),
          new_date: ($n.date | strftime("%Y-%m-%d"))
        }
    ]
  '
)"

count="$(echo "$changes" | jq 'length')"
tooltip="$(
  echo "$changes" | jq -r '
    map("\(.name)  \(.old_date) -> \(.new_date)") | join("\n")
  '
)"

jq -nc \
  --arg text "❆ $count" \
  --arg tooltip "$tooltip" \
  --arg class "outdated" \
  '{text:$text, tooltip:$tooltip, class:$class}'

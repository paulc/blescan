jq '[.[] | select(.uuid | length == 4) | {uuid, name}] | sort_by(.uuid)' 

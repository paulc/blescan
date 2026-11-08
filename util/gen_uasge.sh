#!/bin/sh

cargo build --release

printf '```\n'
target/release/blescan --help
printf '```\n\n'

for cmd in scan monitor enumerate poll write notify dump run sse js; do
    printf '```\n'
    target/release/blescan $cmd --help
    printf '\nJSON Params:\n'
    target/release/blescan --dump-json $cmd 2>&1 | sed -e 's/^/    /'
    printf '```\n\n'
done

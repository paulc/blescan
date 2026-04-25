#!/bin/sh

printf '```\n'
target/release/blescan --help
printf '```\n\n'

for cmd in scan enumerate poll write notify dump run; do
    printf '```\n'
    target/release/blescan $cmd --help
    printf '```\n\n'
done

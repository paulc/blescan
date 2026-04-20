#!/bin/sh

for cmd in scan enumerate poll write notify dump; do
    printf '```\n'
    target/release/blescan $cmd --help
    printf '```\n\n'
done

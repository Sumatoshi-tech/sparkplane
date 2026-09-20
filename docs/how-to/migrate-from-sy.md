# Migration from sy

The source extraction and process bridge are implemented. The live appliance
cutover is not yet released: do not run the fresh-install command over an
existing sy appliance. A namespace migration is not an ordinary upgrade.

## Client configuration import

The following command previews an offline import, without SSH, network traffic,
appliance changes or credential output:

```sh
sparkplane local migrate-client --dry-run --json
```

Once the appliance transition has been verified, repeat with `--yes` instead
of `--dry-run`. Defaults are `$XDG_CONFIG_HOME/sy` and
`$XDG_CONFIG_HOME/sparkplane` (or `$HOME/.config/` when unset). Override them
with absolute `--source` and `--destination` paths when necessary.

Only `spark.toml`, optional `spark-launch.toml`, Spark CA certificates and Spark
credentials are imported. The import preserves credential bytes, converts the
typed launch-state schema, refuses symlinks and existing destinations, and
atomically publishes private files. Source files and unrelated sy settings
remain untouched. Generated coding-client files outside this directory are not
modified by this command. Do not delete them before ownership-aware migration
and launch verification have succeeded.

## Appliance transition gate

The library has tested primitives for an integrity-checked SQLite backup
(including WAL), typed metadata conversion and old-authority verification of
`sparkplane.trust-transition/v1`. They are not a complete host migration runner.
The remaining runner must provide durable interruption recovery, traffic drain,
same-filesystem cache moves, preserved numeric identity and TLS, exact-container
cleanup, rollback before commit, and acceptance before reopening writes.

A transition manifest binds the new release authority to one host identity,
one exact signed release inventory digest, and an expiration. The installed
legacy authority must sign it. Unlock the signing key locally or use a trusted
offline signing station; never send a passphrase in chat or commit a private key.

The optimized engine image and runtime settings must remain unchanged apart
from namespaces. Live acceptance requires full context, tools, streaming,
cancellation, concurrency and matched performance measurements. Keep the legacy
deployment serving until all release and migration gates are ready.

# Migrations

The schema currently lives in a single migration, `0001_initial_schema.sql`.

The application has never been deployed, so no database exists that needs
upgrading. While that remains true, the schema is edited in place (or the
migrations are re-squashed) rather than accreting incremental migrations —
there is no history to preserve. See the header comment in
`0001_initial_schema.sql` for details, including why some `NOT NULL` columns
intentionally have no `DEFAULT`.

> **Pre-deployment note:** Sequential `{num}_{description}` naming is used
> while no production deployment has occurred. Once first deployed, this file
> becomes immutable and subsequent migrations switch to
> `{yyyymmddHHMM}_{description}` naming to avoid merge conflicts. Numbers
> across branches may collide before merge and must be reconciled then.

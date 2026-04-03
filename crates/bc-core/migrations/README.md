# Migrations

Migrations are numbered sequentially. Each milestone claims a block:

| Milestone | Migrations |
|-----------|-----------|
| M1 (core engine) | 0001–0008 |
| M5A (illiquid assets) | 0009–0012 |
| M5 (budgeting) | 0013+ |

> **Pre-deployment note:** Sequential `{num}_{description}` naming is used
> while no production deployment has occurred. Once first deployed, migrations
> will switch to `{yyyymmddHHMM}_{description}` to avoid merge conflicts.
> Numbers across branches may collide before merge and must be reconciled then.

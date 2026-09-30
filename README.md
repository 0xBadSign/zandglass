# Sandglass

**Time-locked inheritance and scheduled payments for shielded Zcash.**

Shielded Zcash has no timelocks, so there is no private way to pass it on. Sandglass gives a vault a time-locked recovery path: your heir can open it only after you stop checking in, and nothing about it ever appears on chain.

> Status: in development for the Zecathon hackathon (build window 2026-09-28 to 2026-10-28). Not audited. Do not use with funds you cannot afford to lose.

## How it works (in short)

1. Your heir creates a heir key (24 words) and gives you the public **heir code**.
2. Sandglass creates a **vault** (a dedicated Zcash seed) and seals it twice: to your heir's code, then to a future moment with [drand timelock encryption](https://github.com/drand/tlock).
3. While you are around, you **flip** the sandglass now and then: your funds move to a fresh vault with a later unlock time, and every older kit now opens an empty vault.
4. If you stop, the time passes and your heir opens the kit, restores the vault in any Zcash wallet and moves the funds.

Every transaction is an ordinary shielded transaction. There is no smart contract, no custody and no fork.

## Repository

| Path | What |
|---|---|
| `crates/sandglass-core` | drand rounds, sealed kits, heir keys |
| `crates/sandglass-wallet` | vault manager and light-client wallet |
| `crates/sandglass-cli` | the `sandglass` command |
| `crates/sandglass-keeper` | blind mailboxes and the hosted playground |
| `web/` | landing page, playground and heir page |
| `docs/` | plan, progress, design and privacy documents |

## License

MIT OR Apache-2.0, at your option.

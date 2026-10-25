# Zecathon submission texts

Paste these into the form on thezecathon.com (the owner submits; see PLAN 2.3 and O-6).
Fill the placeholders marked `<...>` before submitting. The judge code lives in
`.private/` and is pasted only into the form, never into this repository.

## Project name (max 64)

Zandglass: time-locked inheritance for shielded Zcash

## Track

Shielded Payments

## Description (max 2,000 characters)

Shielded Zcash has no timelocks, so there is no private way to pass it on: if you die, your heirs may not even know the funds exist. Zandglass gives a vault a time-locked recovery path that only your heir can use, and only after you stop checking in.

How it works: your heir makes a key in the browser and sends you a public heir code. Zandglass moves your funds into a fresh vault (its own Zcash seed) and seals that seed twice: to the heir code (age, X25519) and to a future drand quicknet round (timelock). While you are around you flip: the funds move to a new vault and a new kit is sealed, so old kits open empty vaults. If you stop, the sand runs out; the heir opens the kit in the browser and restores the vault in any Zcash wallet, with a step-by-step guide. Scheduled payments (an allowance, a salary) use the same mechanism, one vault per installment.

Use it on Monday: a downloadable program (Windows, macOS, Linux) opens an owner panel on your own computer to create plans, fund them by QR code, flip with one click and add calendar reminders. A draft ZIP lets any wallet adopt the kit format.

Why it does not leak: no smart contract, no custody, nothing pre-signed. Every funding, flip and claim is an ordinary shielded transaction built by the standard librustzcash flow at send time (default expiry and anchor, ZIP 317 fee, OvkPolicy::Discard). Kits are constant-size; the mailbox keeper stores only ciphertext it can never open. The privacy review is in docs/PRIVACY.md.

Real: mainnet fund, flip and claim txids, a public mainnet playground, and a live vault that unlocks on 2026-11-12 during judging; testnet runs of installments and of a vault spent in ECC's zcash-devtool. Evidence: docs/evidence/EVIDENCE.md.
Mock: in the playground the server plays the owner with a tiny demo vault (your heir key stays in your browser); timers are minutes. Unaudited; mainnet plans are capped.

Playground judge code: <JUDGE CODE>

## Code repository

https://github.com/0xBadSign/zandglass

## Working demo

https://zandglass.cash/  (landing, playground, heir, claim and owner pages; judge code above)

## Demo video

https://zandglass.cash/media/zandglass-demo-v1.mp4  (1:37, narrated, plays in any browser without sign-in)

## Checklist before SUBMIT

- [x] Repository is public (C-3), 2026-10-25.
- [ ] Demo URL loads and the playground accepts the judge code; the float holds funds.
- [x] Video link plays without sign-in (checked 2026-10-02).
- [x] Mainnet evidence rows exist in `docs/evidence/EVIDENCE.md` (2026-10-01: fund, flip, claim).
- [x] Prize address: not asked in the form; the organizers contact winners on Telegram after judging (site, checked 2026-10-21).
- [ ] The "repository will be public" box is ticked.

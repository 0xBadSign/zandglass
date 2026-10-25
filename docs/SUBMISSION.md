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

How it works: your heir makes a key in the browser and sends you a public heir code. Zandglass moves your funds into a fresh vault (its own Zcash seed) and seals that seed twice: to the heir code (age) and to a future drand round (timelock). While you are around you flip: funds move to a new vault and a new kit is sealed, so old kits open empty vaults. If you stop, the heir opens the kit in the browser and restores the vault in any Zcash wallet.

Use it on Monday: one program (Windows, macOS, Linux) opens an owner panel on your own computer for inheritance, scheduled payments (one vault per installment) and a savings lock that not even you can open early. Fund by QR code, flip with one click, calendar reminders, a paper kit of QR codes for the will envelope. Heirs rehearse with a 60-second practice kit; kits are owner-signed, so a swapped kit is caught. A draft ZIP lets any wallet adopt the format.

Why it does not leak: no contract, no custody, nothing pre-signed. Every funding, flip and claim is an ordinary shielded transaction built by the standard librustzcash flow at send time (default expiry and anchor, ZIP 317 fee, OvkPolicy::Discard). Kits are constant-size; the mailbox keeper stores only ciphertext. Privacy review: docs/PRIVACY.md.

Real: mainnet fund, flip and claim txids, a public mainnet playground, a live vault that unlocks on 2026-11-12 during judging; NU7-ready (tested on testnet). Evidence: docs/evidence/EVIDENCE.md.
Mock: in the playground the server plays the owner with a tiny demo vault (your heir key stays in your browser); timers are minutes. Unaudited; mainnet plans are capped.

Playground judge code: <JUDGE CODE>

## Code repository

https://github.com/0xBadSign/zandglass

## Working demo

https://zandglass.cash/  (landing, playground, heir, claim and owner pages; judge code above)

## Demo video

https://zandglass.cash/media/zandglass-demo-v2.mp4  (1:39, narrated and captioned, plays in any browser without sign-in)

## Checklist before SUBMIT

- [x] Repository is public (C-3), 2026-10-25.
- [x] Demo URL loads and the playground accepts the judge code; the float holds funds (mainnet run 2026-10-26).
- [x] Video link plays without sign-in (v2 checked 2026-10-25).
- [x] Mainnet evidence rows exist in `docs/evidence/EVIDENCE.md` (2026-10-01: fund, flip, claim).
- [x] Prize address: not asked in the form; the organizers contact winners on Telegram after judging (site, checked 2026-10-21).
- [x] The "repository will be public" box is ticked; submitted 2026-10-25.

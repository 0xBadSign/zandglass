# Zecathon submission texts

Paste these into the form on thezecathon.com (the owner submits; see PLAN 2.3 and O-6).
Fill the placeholders marked `<...>` before submitting. The judge code lives in
`.private/` and is pasted only into the form, never into this repository.

## Project name (max 64)

Zandglass: time-locked inheritance for shielded Zcash

## Track

Shielded Payments

## Description (max 2,000 characters)

Shielded Zcash has no timelocks, so there is no private way to pass it on: if you die, your heirs cannot even see that the funds exist. Zandglass gives a vault a time-locked recovery path that only your heir can use, only after you stop checking in.

How it works: your heir makes a key in the browser and sends you a public heir code. Zandglass moves your funds into a dedicated vault (a fresh Zcash seed) and seals that seed twice: to the heir code (age, X25519) and to a future moment (drand quicknet timelock). While you are around you "flip": the funds move to a fresh vault and a new kit is sealed, so old kits open empty vaults. If you stop, the sand runs out and the heir opens the kit in the browser and restores the vault in any Zcash wallet. Installments (cancelable scheduled payments) use the same mechanism.

Why it does not leak: there is no smart contract and nothing pre-signed. Funding, flips and claims are ordinary shielded transactions built by the standard librustzcash flow at send time (default expiry and anchor, ZIP 317 fee, OvkPolicy::Discard), so nothing on chain reveals the plan, the heir or the amounts. Before the unlock nobody can open a kit; after it, only a listed heir. The mailbox keeper stores ciphertext it can never open and logs no requests. The web pages never send the phrase or the seed (checked in automated browser tests).

Real: Zcash testnet and mainnet transactions for funding, flips, installments and claims; a claim restored and spent in ECC's zcash-devtool; kits that open with the reference tle and age tools. Evidence with txids: docs/evidence/EVIDENCE.md.
Mock: in the playground the server plays the owner and holds a tiny demo vault (your heir key stays in your browser); timers are minutes, not months. Unaudited; mainnet plans are capped.

Playground judge code: <JUDGE CODE>

## Code repository

https://github.com/0xnesQuit/zandglass

## Working demo

<https://zandglass.cash/>  (landing, playground, heir and claim pages; judge code above)

## Demo video

<VIDEO URL>

## Checklist before SUBMIT

- [ ] Repository is public (C-3).
- [ ] Demo URL loads and the playground accepts the judge code; the float holds funds.
- [ ] Video link plays without sign-in.
- [ ] Mainnet evidence rows exist in `docs/evidence/EVIDENCE.md` (the description says "testnet and mainnet").
- [ ] Prize address: a shielded (unified) address of the owner, entered in the form.
- [ ] The "repository will be public" box is ticked.

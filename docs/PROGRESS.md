# Sandglass: Progress

Live state of the project. The agent updates it at the end of every work unit, newest first. Plan: `docs/PLAN.md` (version 2). Operating rules: `CLAUDE.md`.

---

## Where we are

| | |
|---|---|
| Last updated | 2026-09-30 |
| Plan version | 2 |
| Current milestone | M0 Foundations and feasibility (starts 2026-10-01) |
| Next task | T0.1 Repository scaffold |
| Blocked / waiting on owner | nothing yet (O-1 and O-4 are due soon; O-5 after the mainnet address exists) |
| Days to deadline | 28 (submissions due 2026-10-28 23:59 UTC; owner submits by 10-27) |
| Health | on track |

---

## Owner actions

| ID | Action | Due | Status |
|---|---|---|---|
| O-1 | GitHub access on this PC (`gh auth login`); repository under the owner's account, private until 10-26 | 2026-10-03 | open |
| O-2 | Server: reuse the existing VPS (install the agent's new SSH public key) or buy a Hetzner CX33 | 2026-10-12 | open |
| O-3 | (Optional) domain | 2026-10-14 | open |
| O-4 | (If needed) testnet TAZ to the agent's development address | 2026-10-02 | open (address not generated yet) |
| O-5 | 0.1–0.2 ZEC to the mainnet demo and evidence wallet (what is left is returned) | 2026-10-08 | open (address not generated yet) |
| O-6 | Save a draft (10-25) and SUBMIT (10-27) with texts from `docs/SUBMISSION.md` and the judge code from `.private/`; prize address; repository-public box | 10-25 / 10-27 | open |
| O-7 | (Optional) video voiceover | 2026-10-24 | open |

---

## Checklist

### M0 Foundations and feasibility (10-01 to 10-03)
- [x] T0.1 Repository scaffold (0ab0ccf)
- [x] T0.2 Toolchain: Rust 1.98.1, age 1.2.1, tle v1.2.0, Go 1.26, protoc (WSL)
- [x] T0.3 Dependency decision (D-011)
- [ ] T0.4 Connectivity (mainnet and testnet lightwalletd, drand)
- [ ] T0.5 Wallets and funds (testnet development wallet; mainnet demo and evidence address for O-5)
- [ ] T0.6 Spike A: vault lifecycle and restore in a third-party wallet
- [ ] T0.7 Spike B: nested encryption and interoperability (Rust, `tle`+`age`, browser)
- [ ] T0.8 Decisions recorded; M0 exit review

### M1 Core and CLI (10-04 to 10-10)
- [ ] T1.1 core::drand
- [ ] T1.2 core::seal, heir keys, kit v1
- [ ] T1.3 sandglass-wallet (vault manager, sweep, split, claim)
- [ ] T1.4 CLI commands
- [ ] T1.5 End to end: testnet then mainnet, with evidence
- [ ] T1.6 CI

### M2 Keeper, heir page, installments (10-11 to 10-16)
- [ ] T2.1 Keeper mailbox
- [ ] T2.2 Heir page (create a key; claim; no secret leaves the page)
- [ ] T2.3 Installments (split, payee auto-claim, cancel)
- [ ] T2.4 Multiple heirs (1 to 3)

### M3 Mainnet playground and deployment (10-17 to 10-21)
- [ ] T3.1 Playground backend (judge code, caps, float)
- [ ] T3.2 Landing and playground UI
- [ ] T3.3 Deploy (server, TLS, Pages, health, backups)
- [ ] T3.4 Browser end-to-end tests and axe
- [ ] T3.5 Live vault on mainnet (unlock about 2026-11-12)

### M4 Polish, docs, video, submission texts (10-22 to 10-25)
- [ ] T4.1 Adversarial security, privacy and UX review; on-chain field comparison; fixes
- [ ] T4.2 Docs set
- [ ] T4.3 Video (2:00 or less)
- [ ] T4.4 docs/SUBMISSION.md
- [ ] T4.5 Release v0.1.0
- [ ] T4.6 Owner draft saved (O-6)

### M5 Final (10-26 to 10-28)
- [ ] T5.1 Repository public; owner SUBMIT by 10-27; tag `zecathon-submission`; freeze
- [ ] T5.2 Buffer

### M6 Judging support (10-29 to 11-23)
- [ ] T6.1 Demo up; float watch; NU7 dependency bump before 11-05 (ops branch); claim the live vault
- [ ] T6.2 Freeze policy respected
- [ ] T6.3 Bounties and judges' questions

### Stretch (only after all Musts)
- [ ] S1 Auto-delivery mode (pre-signed, opt-in, fingerprint warning)
- [ ] S2 Local owner web UI
- [ ] S3 Reminders and alerts
- [ ] S4 k-of-n heirs
- [ ] S5 In-browser sweep for heirs (WASM wallet)
- [ ] S6 FROST escrow with a time-locked refund

---

## Compliance (plan section 2.5)

| ID | Status | Evidence |
|---|---|---|
| C-1 | ok so far | Planning docs written 2026-09-30, inside the window. No code yet. |
| C-2 to C-11 | pending | |

---

## Decisions

| ID | Date | Decision | Why |
|---|---|---|---|
| D-001 | 2026-09-30 | Track: Shielded Payments (revised the same day, from Core & Tooling) | After D-009 the product is a user-facing way to move shielded money privately: inheritance, allowances, subscriptions. It stands out there against POS and payroll entries. The Grand Prize is cross-track. |
| D-002 | 2026-09-30 | Working name: Sandglass | "Heirloom" is used by several crypto inheritance projects. The sandglass metaphor gives the UX its verbs (seal, flip, the sand runs out). A defunct Solana yield protocol used the name until June 2026; there is no conflict on Zcash. |
| D-003 | 2026-09-30 | Mainnet is the product's default network. Testnet is only for development and CI. Mainnet plans are capped at 0.25 ZEC unless `--accept-unaudited`. | Owner instruction: finish and submit on mainnet. The cap protects early users until an audit. |
| D-004 | 2026-09-30 | The playground runs on mainnet: the server plays the owner with a small float (0.001 ZEC per session, judge code, caps); the heir key is generated in the visitor's browser and never leaves it | Judges can use it on mainnet without buying ZEC; the float is the owner's demo money, not strangers' funds; disclosed as the main mock |
| D-005 | 2026-09-30 | Sealed object = the vault seed, encrypted to heir codes (age X25519) and then to a drand quicknet round (tlock) | AND-gate (time and heir); revocable by rolling vaults; upgrade-proof; claim with any wallet; standard formats (no lock-in) |
| D-006 | 2026-09-30 | Judging criteria found in the site's data file: Privacy ("Leaks are disqualifying, not deductions"), Usefulness ("use this on Monday"), Execution ("Working beats ambitious"), Originality ("not repackaged") | Drives D-007, D-009, the privacy review (C-11) and the cut order |
| D-007 | 2026-09-30 | Every judge-facing part (demo, evidence, video) is on mainnet | The rules say nothing about testnet; the criteria favour real use |
| D-008 | 2026-09-30 | Heir key = 24 words, HKDF (`sandglass-heir-v1`), X25519 age identity; heir code = the age recipient string | Standard, portable, openable with the reference `age` tool; phrase backup is familiar to crypto users |
| D-013 | 2026-09-30 | Commits use a repo-local neutral identity (`Sandglass contributors <dev@sandglass.invalid>`) until O-1 gives a GitHub noreply address | C-9: the global git identity carries the owner's real name and email |
| D-012 | 2026-09-30 | drand quicknet verified: chain `52db9ba7…e971`, genesis 1692803367, period 3, scheme `bls-unchained-g1-rfc9380`, public key `83cf0f28…ece45a` | T0.4 |
| D-011 | 2026-09-30 | Build on the released cohort (`zcash_client_backend` 0.24.0, `zcash_client_sqlite` 0.22.0, `zcash_primitives` 0.30.1, `zcash_protocol` 0.10.6, `zcash_keys` 0.16.1, `orchard` 0.15.5, `zip321` 0.9.0). Released crates have no NU7 (still `zcash_unstable`); librustzcash main has `BranchId::Nu7 = 0x77190AD9` but no testnet or mainnet height. After testnet NU7 (10-06), if no release carries the height, continue development on mainnet with small amounts; bump before mainnet NU7 (T6.1). | The sealed-vault design builds only ordinary transactions, so NU7 only matters for the wallet's transaction builder |
| D-010 | 2026-09-30 | Coding subagents run on the Sonnet model; the main agent reviews and integrates | Owner instruction |
| D-009 | 2026-09-30 | Pre-signed transactions (plan v1) demoted to opt-in "auto-delivery" (S1) | Expiry 0 and old anchors are on-chain fingerprints (leaks disqualify); signatures go stale at network upgrades |

---

## Evidence (summary; details in `docs/evidence/EVIDENCE.md`)

None yet.

---

## Session log (newest first)

### 2026-09-30 (2): Plan v2, mainnet first and no on-chain footprint
- The owner asked for mainnet unless the rules allow testnet. The rules, FAQ, resources and the site's data file say nothing about networks, so all judge-facing parts move to mainnet (D-007).
- Found the judging criteria in the site's data file (D-006). "Leaks are disqualifying" means the pre-signed design's fingerprints are unacceptable as a default.
- Redesigned the mechanism around sealed vault seeds with heir keys (D-005, D-008, D-009) and switched the track (D-001).
- Rewrote `PLAN.md` to version 2.
- Next: T0.1.

### 2026-09-30 (1): Planning
- Research done before writing the plan:
  - the hackathon rules, tracks and submission form;
  - past Zcash hackathon winners;
  - comparable products on other chains;
  - ZIP 203 and ZIP 229 facts;
  - Ironwood library versions;
  - zec.rocks endpoints, faucets, drand quicknet and tlock libraries;
  - Hetzner prices.
- Created `PLAN.md` v1, `PROGRESS.md` and `CLAUDE.md`. No code yet (the owner asked for the plan first).

---

## Known issues and technical debt

None yet.

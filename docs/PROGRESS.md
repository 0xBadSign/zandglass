# Zandglass: Progress

Live state of the project. The agent updates it at the end of every work unit, newest first. Plan: `docs/PLAN.md` (version 2). Operating rules: `CLAUDE.md`.

---

## Where we are

| | |
|---|---|
| Last updated | 2026-09-30 |
| Plan version | 2 |
| Current milestone | M0 Foundations and feasibility (starts 2026-10-01) |
| Next task | T2.3 installments, T2.4 multiple heirs (already supported in core and CLI: verify), then M3 landing, playground and deploy; T1.5 mainnet run when O-5 arrives |
| Blocked / waiting on owner | O-5 (0.02 ZEC to the mainnet float address, by 10-08) |
| Days to deadline | 28 (submissions due 2026-10-28 23:59 UTC; owner submits by 10-27) |
| Health | on track |

---

## Owner actions

| ID | Action | Due | Status |
|---|---|---|---|
| O-1 | GitHub access on this PC (`gh auth login`); repository under the owner's account, private until 10-26 | 2026-10-03 | done 2026-09-30: account `0xnesQuit`, private repo `0xnesQuit/zandglass` |
| O-2 | Server: reuse the existing VPS (install the agent's new SSH public key) or buy a Hetzner CX33 | 2026-10-12 | done 2026-10-01: existing VPS reachable with the deploy key (4 vCPU, 6 GB RAM, 50 GB; details in .private/SERVER.md) |
| O-3 | (Optional) domain | 2026-10-14 | done 2026-10-01: `zandglass.cash`, nameservers on Cloudflare |
| O-4 | (If needed) testnet TAZ to the agent's development address | 2026-10-02 | done 2026-09-30: owner sent 1 TAZ |
| O-5 | 0.02 ZEC (0.01 is enough to start) to the mainnet demo and evidence wallet (what is left is returned) | 2026-10-08 | open: NEW address `u1zhzeajwadh8vr6rtjmyf3ykkscyu79vujvych0wxuhpwn450whzymh99cmujj0za86cuy8gk5qymv4dr65ymfc0nhafndczy9qgzukxx` (birthday 3502506). The earlier address is abandoned (D-015): never send to it |
| O-6 | Save a draft (10-25) and SUBMIT (10-27) with texts from `docs/SUBMISSION.md` and the judge code from `.private/`; prize address; repository-public box | 10-25 / 10-27 | open |
| O-7 | (Optional) video voiceover | 2026-10-24 | open |

---

## Checklist

### M0 Foundations and feasibility (10-01 to 10-03)
- [x] T0.1 Repository scaffold (0ab0ccf)
- [x] T0.2 Toolchain: Rust 1.98.1, age 1.2.1, tle v1.2.0, Go 1.26, protoc (WSL)
- [x] T0.3 Dependency decision (D-011)
- [x] T0.4 Connectivity: zec.rocks:443 (main, height 3501837) and testnet.zec.rocks:443 (test, 4423651), lightwalletd v0.5.x; drand D-012
- [x] T0.5 Wallets and funds: 1 TAZ from the owner (faucet), mainnet float address issued (O-5 pending)
- [x] T0.6 Spike A: fund, flip (standard fields) and restore proven on testnet; third-party wallet restore moves to T1.5
- [x] T0.7 Spike B: nested sealing; Rust, tle+age and tlock-js+age-encryption open each other's kits (scripts/interop-kit.sh)
- [x] T0.8 Decisions recorded; M0 exit review (all spikes passed; no plan change needed; known issue: re-sync right before building a sweep, since testnet blocks come about every 18 s and a stale tip expired one attempt)

### M1 Core and CLI (10-04 to 10-10)
- [x] T1.1 core::drand
- [x] T1.2 core::seal, heir keys, kit v1
- [x] T1.3 zandglass-wallet: encrypted store, flip, cancel, claim (split moves to T2.3)
- [x] T1.4 CLI commands (terminal QR not done; low priority)
- [ ] T1.5 End to end: testnet DONE (evidence 2026-09-30); mainnet pending O-5; third-party wallet restore pending
- [ ] T1.6 CI

### M2 Keeper, heir page, installments (10-11 to 10-16)
- [x] T2.1 Keeper mailbox (axum + SQLite, token hashes, no IP logs, CLI --mailbox and claim links)
- [x] T2.2 Heir page: become-an-heir and claim pages; Playwright proves no secret leaves the page; axe clean
- [x] T2.3 Installments (split, payee auto-claim `claim --watch`, cancel) with testnet evidence
- [x] T2.4 Multiple heirs (1 to 3), testnet evidence

### M3 Mainnet playground and deployment (10-17 to 10-21)
- [x] T3.1 Playground backend (judge code, caps, float; offline-tested; live run pending funded float)
- [x] T3.2 Landing and playground UI (mocked-API Playwright flow, leak checks, axe)
- [ ] T3.3 Deploy (server, TLS, Pages, health, backups)
- [ ] T3.4 Browser end-to-end tests and axe
- [ ] T3.5 Live vault on mainnet (unlock about 2026-11-12)

### M4 Polish, docs, video, submission texts (10-22 to 10-25)
- [x] T4.1 Adversarial security and privacy review: no critical; 2 high (flip seed deletion on ambiguous broadcast) and 9 others fixed; PRIVACY.md corrected
- [x] T4.2 Docs set (README, HOW_IT_WORKS, PRIVACY, KIT_FORMAT, KEEPER_API, THIRD_PARTY); plan-vs-code gaps listed in the session log
- [ ] T4.3 Video (2:00 or less)
- [x] T4.4 docs/SUBMISSION.md drafted (1,832 of 2,000 characters; placeholders for demo URL, video URL, judge code)
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
| D-002 | 2026-09-30 | Working name: Sandglass (renamed to Zandglass, D-014) | "Heirloom" is used by several crypto inheritance projects. The sandglass metaphor gives the UX its verbs (seal, flip, the sand runs out). A defunct Solana yield protocol used the name until June 2026; there is no conflict on Zcash. |
| D-003 | 2026-09-30 | Mainnet is the product's default network. Testnet is only for development and CI. Mainnet plans are capped at 0.25 ZEC unless `--accept-unaudited`. | Owner instruction: finish and submit on mainnet. The cap protects early users until an audit. |
| D-004 | 2026-09-30 | The playground runs on mainnet: the server plays the owner with a small float (0.0002 ZEC per session, returned by default; judge code, caps); the heir key is generated in the visitor's browser and never leaves it | Judges can use it on mainnet without buying ZEC; the float is the owner's demo money, not strangers' funds; disclosed as the main mock |
| D-005 | 2026-09-30 | Sealed object = the vault seed, encrypted to heir codes (age X25519) and then to a drand quicknet round (tlock) | AND-gate (time and heir); revocable by rolling vaults; upgrade-proof; claim with any wallet; standard formats (no lock-in) |
| D-006 | 2026-09-30 | Judging criteria found in the site's data file: Privacy ("Leaks are disqualifying, not deductions"), Usefulness ("use this on Monday"), Execution ("Working beats ambitious"), Originality ("not repackaged") | Drives D-007, D-009, the privacy review (C-11) and the cut order |
| D-007 | 2026-09-30 | Every judge-facing part (demo, evidence, video) is on mainnet | The rules say nothing about testnet; the criteria favour real use |
| D-008 | 2026-09-30 | Heir key = 24 words, HKDF (`zandglass-heir-v1`), X25519 age identity; heir code = the age recipient string | Standard, portable, openable with the reference `age` tool; phrase backup is familiar to crypto users |
| D-013 | 2026-09-30 | Commits use a repo-local neutral identity (`Zandglass contributors <dev@zandglass.invalid>`) until O-1; since O-1 commits use the GitHub noreply address of the owner account | C-9: the global git identity carries the owner's real name and email |
| D-012 | 2026-09-30 | drand quicknet verified: chain `52db9ba7…e971`, genesis 1692803367, period 3, scheme `bls-unchained-g1-rfc9380`, public key `83cf0f28…ece45a` | T0.4 |
| D-011 | 2026-09-30 | Build on the released cohort (`zcash_client_backend` 0.24.0, `zcash_client_sqlite` 0.22.0, `zcash_primitives` 0.30.1, `zcash_protocol` 0.10.6, `zcash_keys` 0.16.1, `orchard` 0.15.5, `zip321` 0.9.0). Released crates have no NU7 (still `zcash_unstable`); librustzcash main has `BranchId::Nu7 = 0x77190AD9` but no testnet or mainnet height. After testnet NU7 (10-06), if no release carries the height, continue development on mainnet with small amounts; bump before mainnet NU7 (T6.1). | The sealed-vault design builds only ordinary transactions, so NU7 only matters for the wallet's transaction builder |
| D-010 | 2026-09-30 | Coding subagents run on the Sonnet model; the main agent reviews and integrates | Owner instruction |
| D-015 | 2026-10-01 | The first mainnet float wallet is abandoned and replaced. A WSL path mix-up had created it inside the repository, and its seed was committed (2026-09-30, private repo). It never held funds (balance checked: 0). The new wallet lives in WSL `~/zandglass-evidence/main-float` (0700); `check.sh` now rejects seed-like files and BIP 39-looking lines. The dead seed is still in git history: rewrite history before the repository goes public (owner approval needed for the force push). | Escalated to the owner the same day |
| D-014 | 2026-10-01 | Product renamed Sandglass -> Zandglass (Z for Zcash; keeps the hourglass metaphor). Everything is renamed: crates, binary, env vars (`ZANDGLASS_*`), kit format and heir-key domain strings (test vectors regenerated), kit files (`.zandglass`), GitHub repository. Domain: `zandglass.cash` (O-3). | Owner choice; a "zk" name was rejected because Zandglass makes no zero-knowledge claim of its own |
| D-009 | 2026-09-30 | Pre-signed transactions (plan v1) demoted to opt-in "auto-delivery" (S1) | Expiry 0 and old anchors are on-chain fingerprints (leaks disqualify); signatures go stale at network upgrades |

---

## Evidence (summary; details in `docs/evidence/EVIDENCE.md`)

None yet.

---

## Session log (newest first)

### 2026-10-01 (evening): Owner panel, reminders, lightwalletd fallback, shield logo
- Owner panel `zandglass ui` (local, 127.0.0.1, session key, Host check) with embedded web pages; `zandglass remind` (.ics). Live testnet check through the real UI (`web/e2e-live/owner-live.mjs`): store, plan with claim link, funding, sync, flip (tx `74e19dce2dbdef7718b9a72ccfaa6740e29826a85c3beee3ae6e800ff13626bf`), kit and calendar downloads (e8283bf).
- Found and fixed: cancel right after a flip marked the plan cancelled while the flip's funds were still unconfirmed (invisible to a light client). Cancel now refuses for 2 hours after a flip when the vault looks empty (unit test).
- lightwalletd fallback: 6 public mainnet servers tried in order with a probe (e11a1a7). Testnet has one public server.
- Logo C (shield with an hourglass cut out) in the header and favicon.
- Playwright 40/40; check.sh green; deployed.
- NU7: `zcash_protocol 0.11.0-pre.0` sets testnet NU7 at 4465026 (about 2026-11-01 at the current pace); mainnet has no height yet. The testnet playground stops working at testnet NU7 unless the cohort is bumped; the plan is to run the playground on mainnet (O-5) and bump when the cohort with the mainnet height is released (T6.1, before 11-05).
- Next: O-5 (0.02 ZEC, new address) for T1.5 mainnet evidence, the mainnet playground and the live vault; final video; history rewrite before the repository goes public.

### 2026-10-01 (later): Redesign, clean URLs, rename to Zandglass, first deploy
- Redesign of all pages (canvas sandglass, scroll story, spot-the-transaction, console layout for playground, heir and claim). Playwright 32/32, check.sh green (3231569).
- Clean page addresses `/playground`, `/heir`, `/claim`; old `.html` addresses redirect (1375236, 6fa15eb).
- Stray never-funded mainnet wallet seed removed; seed guard in check.sh (d67482c, D-015).
- Product renamed to Zandglass (386b926, D-014).
- Deploy (T3.3, partly): keeper built on the server as user `zandglass` (`/opt/zandglass`, systemd `zandglass-keeper` on 127.0.0.1:8787, hardened unit), web in `/opt/zandglass/web`, playground on testnet until O-5. cloudflared ingress for `zandglass.cash` and `www` added and validated; waiting for the owner to restart cloudflared and add the DNS CNAMEs (proxied) to the tunnel.
- Owner added the DNS CNAMEs; cloudflared restarted with the owner's approval (other sites checked: all 200). Live at https://zandglass.cash. Full playground rehearsal on the live site passed (EVIDENCE). Pages now send `Cache-Control: no-transform` so Cloudflare does not inject its analytics beacon (verified gone). `.private/deploy.sh` redeploys HEAD.

### 2026-09-30 (3): M0 build start
- T0.1 scaffold (0ab0ccf), T0.2 toolchain, T0.3 dependencies (D-011), T0.4 connectivity (D-012) done. The GitHub private repo `0xnesQuit/zandglass` was created and pushed (O-1 done).
- Spike A (Sonnet subagent, merged):
  - `crates/zandglass-wallet` vault, sync, sweep, restore; the `spike_vault` example;
  - `info` works on mainnet and testnet;
  - testnet vault `utest1p2awydq35puydeurknpfd065drawhcrn783zltwqfydge2fh27ht8ysp4rpgfmgxk4zrn9xlxd44v4spv94ytrdudpfwz3cngykytvwq` (birthday 4423551, seed in WSL `~/zandglass-spikeA/vault1`).
  - Sweep is not yet run with real funds.
- Spike B (Sonnet subagent, merged):
  - drand, heir and seal modules; kit vectors;
  - interoperability in all directions (Rust, `tle`+`age`, tlock-js+age-encryption);
  - the merge needed bip0039 0.12, hkdf 0.12 and sha2 0.10 because `zcash_primitives` pins `crypto-common =0.2.0-rc.1`.
- Mainnet demo and evidence wallet created (seed in WSL `~/zandglass-evidence/main-float`); its address was handed to the owner for O-5.
- The Valar testnet faucet (API, no bot check) returned `global_cap` (retry after about 9 h, around 2026-10-01 06:00 UTC).
- `check.sh`: 24 tests plus all gates pass on `main`.
- Next: retry the faucet, then T0.6 (fund, sweep, restore). Meanwhile M1 T1.3 hardening (encrypted seed storage) and T1.4 CLI.

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

- In agent worktrees, git does not work from WSL (the `.git` file holds a Windows path), so the english-only and secrets greps in `check.sh` pass vacuously there. Always run the final `check.sh` on `main`.
- The `spike_vault` example stores seeds in plaintext (spike only). T1.3 replaces this with age-passphrase encryption.
- `tlock_age` pulls in its own `age` 0.11 beside `age` 0.12.1 (bytes only cross the boundary).
- `drand_core` uses `ureq`; a future WASM build of core must avoid its HTTP client.
- Verified 2026-10-01: third-party restore (zcash-devtool) logged in evidence.
- 2026-10-01: the docs agent found 16 places where the plan and the code differ. Open follow-ups: installments and `claim --watch` (T2.3 agent running); Tor is not implemented (`--server` exists instead of `--lightwalletd`); `sync_required` is unused; the keeper also sees plan id, epoch and creation time (now in PRIVACY R2); claim link path unified on `claim.html` (2026-10-01); the R5 pool-transition note needs review in T4.1.
- 2026-10-01: disk incident. C: had 1.4 GB free because of per-agent cargo target dirs (66 GB). They were deleted, the vhdx trimmed (37 GB free), and CLAUDE.md now requires one shared target dir.

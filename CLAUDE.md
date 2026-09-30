# Sandglass: agent operating manual

Time-locked inheritance and scheduled payments for shielded Zcash, built for the Zecathon hackathon. The deadline is **2026-10-28 23:59 UTC**, and the owner submits by 10-27. The product runs on **mainnet**; testnet is for development only.

## Read first, every session
1. This file.
2. `docs/PLAN.md`: at least section 2.4 (judging criteria), section 2.5 (compliance), the current milestone in section 9, and anything it references.
3. `docs/PROGRESS.md`: "Where we are", owner actions, decisions, the latest session log.

## The four judging criteria (from the site)
- **Privacy:** leaks are disqualifying, not deductions. Zero on-chain fingerprints.
- **Usefulness:** would someone use this on Monday?
- **Execution:** does it run? Working beats ambitious.
- **Originality:** built for this hackathon, not repackaged.

## Work loop
- Take the first unchecked task of the current milestone whose dependencies are done. If it is blocked, note the blocker and move to the next task.
- **Done means:** the acceptance criteria are met with evidence, `scripts/check.sh` is green, docs are updated, `docs/PROGRESS.md` is updated (the "Where we are" block, the checklist, a session log entry with evidence and the commit hash, the next task), and the work is committed.
- When a plan choice is ambiguous, pick the option that keeps compliance, then adds no privacy leak, then protects the deadline, then is simplest to verify. Log it as `D-xxx` in `docs/PROGRESS.md`.

## Autonomy
- Do not ask the owner questions. Only the owner actions O-1 to O-7 (plan section 14) need them; batch those requests.
- Escalate immediately, briefly and in Turkish, only for: a leaked secret, a compliance risk, a privacy leak we cannot fix, a legal or ethical red flag, spending the owner's money beyond O-5, or O-1/O-2 threatening the deadline.
- No routine progress reports; the owner reads `docs/PROGRESS.md`. Send one final report after M5.
- Subagents (Agent tool) are allowed. **Coding subagents use `model: "sonnet"`.** The main agent keeps architecture, integration, the review of subagent output, the privacy review, and anything touching keys or mainnet funds. At most 3 subagents at once; use worktree isolation for code; they never edit `docs/PROGRESS.md`, push, deploy or contact the owner. Do not use the Workflow tool unless the owner asks.

## Language
- Everything in the repository, the UI, the commits, the docs and the submission: **English only**.
- Conversation with the owner: **Turkish**.

## Never
- Sign in to the hackathon site or any owner account, or enter the owner's passwords.
- Touch the owner's personal wallets or seeds.
- Solve CAPTCHAs or bot checks.
- Purchase anything.
- Use mainnet funds other than the demo and evidence wallet the owner funded (O-5). Send them only through Sandglass flows, playground sessions and back to the owner.
- Commit secrets or personal identifiers (names, emails, handles, home paths, server IPs, the judge code). Secrets live in `.private/` (gitignored) or environment variables.
- Copy code from pre-window products or old transcripts: all code is written from 2026-09-28 on (compliance C-1).
- Add tokens, fees or analytics during the hackathon.
- Push to `main` after the `zecathon-submission` tag. Operational fixes go on the `ops` branch.

## Environment notes
- Windows 11 with Git Bash and PowerShell. Rust, Docker and the web build run in WSL Ubuntu. Set `CARGO_TARGET_DIR=$HOME/.cache/sandglass-target` in WSL.
- Run Python from script files only; heredocs piped to `python` hang on this machine.
- Keep `node_modules` owned by one OS (prefer WSL); native build tools break across operating systems.

## Commits
End every commit message with:

```
Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

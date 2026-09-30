# Evidence

Each row is an on-chain or reproducible proof. All times are UTC.

| Date | Network | What it proves | Reference |
|---|---|---|---|
| 2026-09-30 | testnet | A vault seed is funded; funds land in the Ironwood pool | faucet tx `d49a3bbceb9766194bbe9e8c184677b74c00ce2f419318c7adc7e17bcd571ea1` (block 4423675) |
| 2026-09-30 | testnet | A flip: vault #1 swept to a fresh vault #2 with standard fields. Target height 4423783, anchor height 4423780, expiry 4423823 (target + 40), fee 10,000 zat (ZIP 317), `OvkPolicy::Discard` | tx `cbc4dcbab10480fa6dc7b67e575fe439a6288292191f391e844245abbe66e0c3` |
| 2026-09-30 | testnet | A vault seed restored from 24 words and its birthday height (4423623) into a fresh wallet sees the funds (0.9999 TAZ) | local run: `spike_vault restore` then `sync` |
| 2026-09-30 | offline + drand quicknet | One kit opens with Rust, with the reference `tle` + `age` CLIs, and with `tlock-js` + `age-encryption`, in every direction | `scripts/interop-kit.sh` |

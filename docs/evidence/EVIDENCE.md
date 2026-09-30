# Evidence

Each row is an on-chain or reproducible proof. All times are UTC.

| Date | Network | What it proves | Reference |
|---|---|---|---|
| 2026-09-30 | testnet | A vault seed is funded; funds land in the Ironwood pool | faucet tx `d49a3bbceb9766194bbe9e8c184677b74c00ce2f419318c7adc7e17bcd571ea1` (block 4423675) |
| 2026-09-30 | testnet | A flip: vault #1 swept to a fresh vault #2 with standard fields. Target height 4423783, anchor height 4423780, expiry 4423823 (target + 40), fee 10,000 zat (ZIP 317), `OvkPolicy::Discard` | tx `cbc4dcbab10480fa6dc7b67e575fe439a6288292191f391e844245abbe66e0c3` |
| 2026-09-30 | testnet | A vault seed restored from 24 words and its birthday height (4423623) into a fresh wallet sees the funds (0.9999 TAZ) | local run: `spike_vault restore` then `sync` |
| 2026-09-30 | offline + drand quicknet | One kit opens with Rust, with the reference `tle` + `age` CLIs, and with `tlock-js` + `age-encryption`, in every direction | `scripts/interop-kit.sh` |
| 2026-09-30 | testnet | **End-to-end legacy flow with the `sandglass` CLI** (`scripts/e2e-testnet.sh`, plan `3166779e`, timer 6 min): fund vault epoch 1 | tx `c62ad1cbd16bc238ed78349999042db89990cdef992efc408f5d2b55e1fb8d68` (expiry = target + 40, fee 10,000 zat) |
| 2026-09-30 | testnet | Flip: epoch 1 swept to a fresh vault (epoch 2) and re-sealed | tx `d14a2d951681f8a53011cde9c6de26755b279c157212c1b8e598de41ce0a1d62` |
| 2026-09-30 | testnet | The owner stays silent; after the unlock (21:45:27 UTC) the heir opens the epoch 2 kit with the heir phrase and sweeps 0.9997 TAZ to their own address | tx `5685741452b15ae0a8f9c458c6fa4fc1b6d6cfe45a3b2c71eebc80604c5a0407` |
| 2026-09-30 | testnet | Revocation: the old epoch 1 kit opens, but its vault is empty (`vault_empty`) | transcript of the same run |

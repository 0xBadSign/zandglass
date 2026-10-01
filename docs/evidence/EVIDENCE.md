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
| 2026-10-01 | testnet | **Installments**: one ordinary transaction splits a funding vault into 3 installment vaults of 0.0101 TAZ | tx `81b87c16b6f20e2a013e1e1cb0334126ae054c76abfff5fed8b61bb1ccee90f6` |
| 2026-10-01 | testnet | The payee claims each installment automatically with `claim --watch --links-file` and receives exactly 0.01 TAZ each | txs `2fcd237e5450d0ff5b10a0e15d7f3e9d936cec6ac75135b8f6a94cfa6f1387ce`, `0b88aa7de3f76a24af020915a3d31d8a6f5ae348ccd2174a9990d7fb1ad35767`, `918e07479c77d20106fd838a6b7c8e40b4825e9a6fea53353aa37964653cb422` |
| 2026-10-01 | testnet | A second schedule is cancelled after its first installment: the remaining vaults sweep back and the next kit opens an empty vault | split `be1a58bcfa5b0330df5836e8476a64c1e1a53ba74382f863229c269fbb252eb0`, claim `598b562451b823cbc68399477d0c99941e98d1517d16b35988eaca6a120d1b3f`, cancel `cb34e1bc8fa003a553734980608681f9a3cfe68039887f9240f494be2565fc60` |
| 2026-10-01 | testnet | Three heirs on one kit: heirs 1 and 3 can open it, heir 2 opens and sweeps it | tx `f59aa6503d7a362f43bff057e31ff4b192941a176f22ab5c405f01b502a807a6` |

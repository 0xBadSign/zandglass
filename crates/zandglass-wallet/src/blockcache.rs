//! In-memory compact block cache used by the light-client sync loop.

use std::{collections::BTreeMap, convert::Infallible, sync::Mutex};

use async_trait::async_trait;
use zcash_client_backend::{
    data_api::{
        chain::{error, BlockCache, BlockSource},
        scanning::ScanRange,
    },
    proto::compact_formats::CompactBlock,
};
use zcash_protocol::consensus::BlockHeight;

/// A [`BlockCache`] that keeps compact blocks in memory, keyed by height.
///
/// `sync::run` downloads a batch, scans it, then deletes it, so the cache
/// never holds more than one batch.
#[derive(Default)]
pub struct MemBlockCache {
    blocks: Mutex<BTreeMap<u32, CompactBlock>>,
}

impl MemBlockCache {
    /// Creates an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u32, CompactBlock>> {
        self.blocks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn height_of(block: &CompactBlock) -> u32 {
    u32::try_from(block.height).unwrap_or(u32::MAX)
}

impl BlockSource for MemBlockCache {
    type Error = Infallible;

    fn with_blocks<F, WalletErrT>(
        &self,
        from_height: Option<BlockHeight>,
        limit: Option<usize>,
        mut with_block: F,
    ) -> Result<(), error::Error<WalletErrT, Self::Error>>
    where
        F: FnMut(CompactBlock) -> Result<(), error::Error<WalletErrT, Self::Error>>,
    {
        let start = from_height.map(u32::from).unwrap_or(0);
        let selected: Vec<CompactBlock> = self
            .lock()
            .range(start..)
            .take(limit.unwrap_or(usize::MAX))
            .map(|(_, block)| block.clone())
            .collect();
        for block in selected {
            with_block(block)?;
        }
        Ok(())
    }
}

#[async_trait]
impl BlockCache for MemBlockCache {
    fn get_tip_height(
        &self,
        range: Option<&ScanRange>,
    ) -> Result<Option<BlockHeight>, Self::Error> {
        let blocks = self.lock();
        let tip = match range {
            Some(range) => {
                let r = range.block_range();
                blocks
                    .range(u32::from(r.start)..u32::from(r.end))
                    .next_back()
                    .map(|(h, _)| *h)
            }
            None => blocks.keys().next_back().copied(),
        };
        Ok(tip.map(BlockHeight::from_u32))
    }

    async fn read(&self, range: &ScanRange) -> Result<Vec<CompactBlock>, Self::Error> {
        let r = range.block_range();
        Ok(self
            .lock()
            .range(u32::from(r.start)..u32::from(r.end))
            .map(|(_, block)| block.clone())
            .collect())
    }

    async fn insert(&self, compact_blocks: Vec<CompactBlock>) -> Result<(), Self::Error> {
        let mut blocks = self.lock();
        for block in compact_blocks {
            blocks.insert(height_of(&block), block);
        }
        Ok(())
    }

    async fn delete(&self, range: ScanRange) -> Result<(), Self::Error> {
        let r = range.block_range();
        let mut blocks = self.lock();
        let doomed: Vec<u32> = blocks
            .range(u32::from(r.start)..u32::from(r.end))
            .map(|(h, _)| *h)
            .collect();
        for h in doomed {
            blocks.remove(&h);
        }
        Ok(())
    }
}

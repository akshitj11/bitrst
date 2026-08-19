//! Disk-backed block storage with atomic writes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use hex::ToHex;

use crate::block::Block;
use crate::wire::DecodeError;

use super::{BlockStore, StoreError};

/// On-disk block store rooted at a caller-supplied directory.
///
/// Blocks are stored as one file per block hash using a fixed 64-character
/// lowercase hex filename. Writes are committed atomically via a same-directory
/// temporary file, `fsync`, and `rename`.
#[derive(Debug)]
pub struct FileBlockStore {
    root: PathBuf,
}

impl FileBlockStore {
    /// Opens or creates a block store at `root`.
    ///
    /// Stale `*.tmp` residue from interrupted writes is removed on open.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)
            .map_err(|source| StoreError::io("create store directory", source))?;
        let store = Self { root };
        store.cleanup_temp_files()?;
        Ok(store)
    }

    /// Returns the configured storage root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn block_path(&self, hash: &[u8; 32]) -> PathBuf {
        self.root.join(hash.encode_hex::<String>())
    }

    fn temp_path(&self, hash: &[u8; 32]) -> PathBuf {
        self.root
            .join(format!("{}.tmp", hash.encode_hex::<String>()))
    }

    fn cleanup_temp_files(&self) -> Result<(), StoreError> {
        let entries = fs::read_dir(&self.root)
            .map_err(|source| StoreError::io("read store directory", source))?;
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::io("read store entry", source))?;
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                let _ = fs::remove_file(entry.path());
                continue;
            };
            if name.ends_with(".tmp") {
                fs::remove_file(entry.path())
                    .map_err(|source| StoreError::io("remove stale temp block file", source))?;
                continue;
            }
            if !is_valid_block_filename(name) {
                fs::remove_file(entry.path()).map_err(|source| {
                    StoreError::io("remove invalid block store residue", source)
                })?;
            }
        }
        Ok(())
    }

    fn sync_dir(&self) -> Result<(), StoreError> {
        let dir = File::open(&self.root)
            .map_err(|source| StoreError::io("open store directory for sync", source))?;
        dir.sync_all()
            .map_err(|source| StoreError::io("sync store directory", source))?;
        Ok(())
    }
}

fn is_valid_block_filename(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

impl BlockStore for FileBlockStore {
    fn put_block(&mut self, block: &Block) -> Result<(), StoreError> {
        let hash = block.hash();
        let path = self.block_path(&hash);
        let temp_path = self.temp_path(&hash);

        if let Some(parent) = path.parent() {
            if parent != self.root.as_path() {
                return Err(StoreError::InvalidPath);
            }
        }

        let bytes = block.serialize();
        {
            let mut file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&temp_path)
                .map_err(|source| StoreError::io("open temp block file", source))?;
            file.write_all(&bytes)
                .map_err(|source| StoreError::io("write temp block file", source))?;
            file.sync_all()
                .map_err(|source| StoreError::io("sync temp block file", source))?;
        }

        fs::rename(&temp_path, &path)
            .map_err(|source| StoreError::io("rename temp block file", source))?;
        self.sync_dir()?;
        Ok(())
    }

    fn get_block(&self, hash: &[u8; 32]) -> Result<Option<Block>, StoreError> {
        let path = self.block_path(hash);
        if !path.is_file() {
            return Ok(None);
        }

        let mut bytes = Vec::new();
        File::open(&path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|source| StoreError::io("read block file", source))?;

        let block = Block::deserialize(&bytes).map_err(|error| match error {
            DecodeError::Truncated { .. } | DecodeError::TrailingBytes { .. } => {
                StoreError::corrupt("truncated block file")
            }
            DecodeError::LimitExceeded { .. } => StoreError::corrupt("oversized block file"),
            other => StoreError::corrupt(format!("malformed block file: {other}")),
        })?;

        let actual = block.hash();
        if actual != *hash {
            return Err(StoreError::HashMismatch {
                expected: *hash,
                actual,
            });
        }

        Ok(Some(block))
    }

    fn commit(&mut self) -> Result<(), StoreError> {
        self.sync_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::FileBlockStore;
    use crate::block::{Block, BlockHeader};
    use crate::pow::Target;
    use crate::store::{BlockStore, StoreError};
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    const TEST_BITS: u32 = 0x1f00_ffff;
    const NETWORK_TIME: u32 = 1_231_006_505;

    fn sample_block() -> Block {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: [0u8; 32],
            merkle_root: [0u8; 32],
            time: NETWORK_TIME,
            bits: TEST_BITS,
            nonce: 0,
        };
        let mut block = Block::coinbase(header, 0, 50_0000_0000);
        let target = Target::from_bits(TEST_BITS).expect("bits");
        while !target.meets(&block.header.hash()) {
            block.header.nonce = block.header.nonce.wrapping_add(1);
        }
        block
    }

    fn store_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("blocks")
    }

    #[test]
    fn persists_block_across_reopen() {
        let dir = tempdir().expect("tempdir");
        let block = sample_block();
        let hash = block.hash();

        {
            let mut store = FileBlockStore::new(store_path(&dir)).expect("open");
            store.put_block(&block).expect("put");
            store.commit().expect("commit");
        }

        let store = FileBlockStore::new(store_path(&dir)).expect("reopen");
        let loaded = store.get_block(&hash).expect("get").expect("some");
        assert_eq!(loaded, block);
    }

    #[test]
    fn overwrite_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        let block = sample_block();
        let hash = block.hash();

        let mut store = FileBlockStore::new(store_path(&dir)).expect("open");
        store.put_block(&block).expect("put first");
        store.put_block(&block).expect("put second");
        store.commit().expect("commit");

        let loaded = store.get_block(&hash).expect("get").expect("some");
        assert_eq!(loaded, block);
    }

    #[test]
    fn missing_block_returns_none() {
        let dir = tempdir().expect("tempdir");
        let store = FileBlockStore::new(store_path(&dir)).expect("open");
        assert!(store.get_block(&[9u8; 32]).expect("get").is_none());
    }

    #[test]
    fn corrupt_file_returns_contextual_error() {
        let dir = tempdir().expect("tempdir");
        let block = sample_block();
        let hash = block.hash();
        let path = store_path(&dir);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join(hex::encode(hash)), b"not a block").expect("write corrupt");

        let store = FileBlockStore::new(&path).expect("open");
        let err = store.get_block(&hash).expect_err("corrupt");
        assert!(matches!(err, StoreError::Corrupt { .. }));
    }

    #[test]
    fn hash_mismatch_on_read_returns_error() {
        let dir = tempdir().expect("tempdir");
        let block = sample_block();
        let wrong_hash = [1u8; 32];
        let path = store_path(&dir);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join(hex::encode(wrong_hash)), block.serialize()).expect("write");

        let store = FileBlockStore::new(&path).expect("open");
        let err = store.get_block(&wrong_hash).expect_err("mismatch");
        assert!(matches!(err, StoreError::HashMismatch { .. }));
    }

    #[test]
    fn cleans_temp_residue_on_open() {
        let dir = tempdir().expect("tempdir");
        let block = sample_block();
        let hash = block.hash();
        let path = store_path(&dir);
        fs::create_dir_all(&path).expect("mkdir");
        let temp = path.join(format!("{}.tmp", hex::encode(hash)));
        fs::write(&temp, b"partial").expect("write temp");
        assert!(temp.is_file());

        FileBlockStore::new(&path).expect("open cleans temp");
        assert!(!temp.exists());
    }

    #[test]
    fn cleans_invalid_residue_filenames_on_open() {
        let dir = tempdir().expect("tempdir");
        let path = store_path(&dir);
        fs::create_dir_all(&path).expect("mkdir");
        fs::write(path.join("partial"), b"leftover").expect("write");
        fs::write(path.join("ZZ"), b"short").expect("write short");

        FileBlockStore::new(&path).expect("open cleans residue");
        assert!(!path.join("partial").exists());
        assert!(!path.join("ZZ").exists());
    }
}

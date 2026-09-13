//! RNode key-value store layout (port of `storage/RNodeKeyValueStoreManager.scala`).

use std::path::{Path, PathBuf};

use rchain_shared::lmdb::LmdbDirStoreManager;
use rchain_shared::refined::ShardId;
use rchain_shared::store_manager::{Db, LmdbEnvConfig, GB, TB};

/// The file recording which shard owns a data directory, written on first boot and checked on every
/// later one. Because the primary membership keeps the data directory root while every additional
/// shard nests under `shard/…` (see [`shard_data_dir`]), reordering memberships would otherwise
/// silently re-point a shard at another shard's chain. See `node`'s `check_shard_data_dir`.
pub const SHARD_ID_MARKER: &str = "shard-id";

/// The data directory of the shard at `index` in the node's membership list.
///
/// The primary membership (index 0) keeps `root` itself, so every existing single-shard deployment
/// finds its data exactly where it left it; additional memberships nest under `shard/` with one path
/// segment per shard-id segment. [`ShardId::segments`] is reused for the path so the layout cannot
/// collide the way a `/`-to-`_` slug would.
pub fn shard_data_dir(root: &Path, index: usize, shard_id: &ShardId) -> PathBuf {
    if index == 0 {
        return root.to_path_buf();
    }
    shard_id
        .segments()
        .into_iter()
        .fold(root.join("shard"), |path, segment| path.join(segment))
}

/// The RNode DB → LMDB environment mapping (port of `rnodeDbMapping`).
///
/// Keys with the same environment name share one LMDB file.
pub fn rnode_db_mapping() -> Vec<(Db, LmdbEnvConfig)> {
    vec![
        // Block storage
        (
            Db::new("blocks"),
            LmdbEnvConfig::new("blockstorage", 1 * TB),
        ),
        // Block metadata storage
        (
            Db::new("block-metadata"),
            LmdbEnvConfig::new("dagstorage", 100 * GB),
        ),
        (
            Db::new("fringe-data"),
            LmdbEnvConfig::new("dagstorage", 100 * GB),
        ),
        (
            Db::new("finalized-store"),
            LmdbEnvConfig::new("dagstorage", 100 * GB),
        ),
        // Deploys from blocks
        (
            Db::new("deploy-index"),
            LmdbEnvConfig::new("dagstorage", 100 * GB),
        ),
        // Runtime mergeable store (cache of mergeable channels for block-merge)
        (
            Db::new("mergeable-channel-cache"),
            LmdbEnvConfig::new("dagstorage", 100 * GB),
        ),
        // Deploys waiting to be added
        (
            Db::new("deploy-pool"),
            LmdbEnvConfig::new("deploypoolstorage", 1 * GB),
        ),
        // Reporting (trace) cache
        (
            Db::new("reporting-cache"),
            LmdbEnvConfig::new("reporting", 10 * TB),
        ),
        // On-chain RSpace (Rholang state); history and roots share one environment.
        (
            Db::new("rspace-history"),
            LmdbEnvConfig::new("rspace/history", 1 * TB),
        ),
        (
            Db::new("rspace-roots"),
            LmdbEnvConfig::new("rspace/history", 1 * TB),
        ),
        (
            Db::new("rspace-cold"),
            LmdbEnvConfig::new("rspace/cold", 1 * TB),
        ),
        // Transaction store
        (
            Db::new("transaction"),
            LmdbEnvConfig::new("transaction", 1 * GB),
        ),
        // Evaluator RSpace (Rholang state)
        (
            Db::new("eval-history"),
            LmdbEnvConfig::new("eval/history", 1 * TB),
        ),
        (
            Db::new("eval-roots"),
            LmdbEnvConfig::new("eval/history", 1 * TB),
        ),
        (
            Db::new("eval-cold"),
            LmdbEnvConfig::new("eval/cold", 1 * TB),
        ),
    ]
}

/// Open the RNode LMDB store manager over `dir_path` (port of `RNodeKeyValueStoreManager.apply`).
///
/// Databases are distributed across LMDB environments (files) per [`rnode_db_mapping`]; the
/// environments are opened lazily on first access.
pub fn rnode_key_value_store_manager(dir_path: &Path) -> LmdbDirStoreManager {
    LmdbDirStoreManager::new(dir_path, rnode_db_mapping())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchain_shared::store_manager::KeyValueStoreManager;

    /// The primary membership keeps the data-directory root (so existing deployments are
    /// unchanged); each additional shard nests by shard-id segment.
    #[test]
    fn shard_data_dirs_are_the_root_for_the_primary_and_nested_afterwards() {
        let root = Path::new("/var/lib/rnode");
        let primary = ShardId::try_from("/root".to_string()).unwrap();
        let child = ShardId::try_from("/root/child".to_string()).unwrap();
        let grandchild = ShardId::try_from("/root/child/leaf".to_string()).unwrap();

        assert_eq!(shard_data_dir(root, 0, &primary), PathBuf::from(root));
        assert_eq!(
            shard_data_dir(root, 1, &child),
            Path::new("/var/lib/rnode/shard/root/child")
        );
        assert_eq!(
            shard_data_dir(root, 2, &grandchild),
            Path::new("/var/lib/rnode/shard/root/child/leaf")
        );
        // Distinct shard ids never share a directory — even one whose segments could collide under a
        // naive `/`-to-`_` slug.
        assert_ne!(
            shard_data_dir(root, 1, &ShardId::try_from("/a/b".to_string()).unwrap()),
            shard_data_dir(root, 1, &ShardId::try_from("/a_b".to_string()).unwrap())
        );
    }

    #[test]
    fn mapping_has_expected_databases() {
        let mapping = rnode_db_mapping();
        let ids: Vec<&str> = mapping.iter().map(|(db, _)| db.id.as_str()).collect();
        assert!(ids.contains(&"blocks"));
        assert!(ids.contains(&"rspace-history"));
        assert!(ids.contains(&"mergeable-channel-cache"));
        // History and roots share an environment name.
        let history = mapping
            .iter()
            .find(|(db, _)| db.id == "rspace-history")
            .map(|(_, c)| c.name.clone());
        let roots = mapping
            .iter()
            .find(|(db, _)| db.id == "rspace-roots")
            .map(|(_, c)| c.name.clone());
        assert_eq!(history, roots);
    }

    #[tokio::test]
    async fn store_manager_round_trips() {
        let dir = std::env::temp_dir().join(format!(
            "rchain-rnode-kvm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = rnode_key_value_store_manager(&dir);
        let store = manager.store("deploy-pool").await.unwrap();
        {
            let mut kv = store.lock().await;
            kv.put(vec![(b"k".to_vec(), b"v".to_vec())]).unwrap();
        }
        {
            let kv = store.lock().await;
            assert_eq!(kv.get(&[b"k".to_vec()]).unwrap(), vec![Some(b"v".to_vec())]);
        }
        manager.shutdown().await;
        drop(manager);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

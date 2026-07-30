use chrono::{DateTime, Utc};
use eyre::{eyre, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::chain::Chain;

/// A single NFT/item during listing, before any download happens.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    /// Stable identifier for this item, independent of name collisions.
    /// Shape: "<contract_or_alias>:<token_id>".
    pub id: String,
    pub name: String,
    pub collection_name: Option<String>,
    pub image_url: Option<String>,
    pub mime_type: Option<String>,
    /// Size in bytes, if the source API reports
    pub size_bytes: Option<u64>,
}

/// Per-item bookkeeping persisted in the manifest, layered on top of the
/// listing data above.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemState {
    #[serde(flatten)]
    pub item: Item,
    pub selected: bool,
    pub downloaded: bool,
    pub file_name: Option<String>,
    pub first_seen: DateTime<Utc>,
}

/// The full state of a collection (an account, or a user-defined group),
/// persisted as JSON at `<folder>/.artfs/state.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CollectionState {
    /// Human-facing identifier: address, ENS name, or a user-chosen
    /// collection name.
    pub collection_id: String,
    pub chain: Option<Chain>,
    /// Which backend produced the current item list
    pub source: String,
    pub last_synced: Option<DateTime<Utc>>,
    /// Keyed by Item::id.
    pub items: HashMap<String, ItemState>,
}

impl CollectionState {
    const STATE_DIR: &'static str = ".artfs";
    const STATE_FILE: &'static str = "state.json";

    fn state_path(folder: &Path) -> PathBuf {
        folder.join(Self::STATE_DIR).join(Self::STATE_FILE)
    }

    /// Load existing state for a folder, or start a fresh one if none exists yet.
    pub fn load_or_new(
        folder: &Path,
        collection_id: &str,
        chain: Option<Chain>,
        source: &str,
    ) -> Result<Self> {
        let path = Self::state_path(folder);
        if path.is_file() {
            let raw = std::fs::read_to_string(&path)
                .map_err(|err| eyre!("Failed to read state file {path:?}: {err}"))?;
            let state: CollectionState = serde_json::from_str(&raw)
                .map_err(|err| eyre!("Failed to parse state file {path:?}: {err}"))?;
            Ok(state)
        } else {
            Ok(CollectionState {
                collection_id: collection_id.to_string(),
                chain,
                source: source.to_string(),
                last_synced: None,
                items: HashMap::new(),
            })
        }
    }

    pub fn save(&self, folder: &Path) -> Result<()> {
        let dir = folder.join(Self::STATE_DIR);
        std::fs::create_dir_all(&dir).map_err(|err| eyre!("Failed to create {dir:?}: {err}"))?;
        let path = Self::state_path(folder);
        let raw = serde_json::to_string_pretty(self)
            .map_err(|err| eyre!("Failed to serialize state: {err}"))?;
        std::fs::write(&path, raw).map_err(|err| eyre!("Failed to write {path:?}: {err}"))?;
        Ok(())
    }

    /// Merge a freshly fetched item list into the manifest: known items get
    /// their listing data refreshed brand new items are added as unselected
    /// and undownloaded. Nothing already downloaded is ever removed,
    /// even if it no longer appears upstream (e.g. transferred away).
    /// manifest tracks what's on disk, not just current ownership.
    ///
    /// Returns the ids of items that are new since the last sync.
    pub fn merge_listed(&mut self, listed: Vec<Item>) -> Vec<String> {
        let mut new_ids = Vec::new();
        let now = Utc::now();

        for item in listed {
            match self.items.get_mut(&item.id) {
                Some(existing) => {
                    existing.item = item;
                }
                None => {
                    new_ids.push(item.id.clone());
                    self.items.insert(
                        item.id.clone(),
                        ItemState {
                            item,
                            selected: false,
                            downloaded: false,
                            file_name: None,
                            first_seen: now,
                        },
                    );
                }
            }
        }

        new_ids
    }

    /// Items that are selected but not yet successfully downloaded.
    pub fn pending(&self) -> Vec<&ItemState> {
        self.items
            .values()
            .filter(|i| i.selected && !i.downloaded)
            .collect()
    }

    pub fn mark_downloaded(&mut self, id: &str, file_name: String) {
        if let Some(item) = self.items.get_mut(id) {
            item.downloaded = true;
            item.file_name = Some(file_name);
        }
    }

    pub fn set_selected(&mut self, ids: &[String]) {
        for state in self.items.values_mut() {
            state.selected = ids.contains(&state.item.id);
        }
    }

    pub fn touch_synced(&mut self) {
        self.last_synced = Some(Utc::now());
    }
}

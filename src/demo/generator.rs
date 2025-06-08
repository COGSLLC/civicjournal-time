#![cfg(feature = "demo")]

use crate::demo::DemoConfig;
use crate::{CJResult, api::async_api::{Journal, PageContentHash}};
use crate::demo::postgres::PgClient;
use serde_json::{Value, json};
use fake::{Fake, faker::{name::en::Name, lorem::en::Sentence}};
use rand::{rngs::StdRng, Rng, SeedableRng};
use chrono::{DateTime, Utc};
use hex;

/// Generates synthetic journal leaves for the simulator.
pub struct LeafGenerator {
    config: DemoConfig,
    rng: StdRng,
    pg: Option<PgClient>,
    journal: Journal,
    prev_hash: Option<[u8; 32]>,
}

impl LeafGenerator {
    /// Create a new generator from the demo configuration and journal.
    pub async fn new(config: DemoConfig, journal: Journal, wipe_db: bool) -> CJResult<Self> {
        let rng = StdRng::seed_from_u64(config.seed);
        let pg = if let Some(url) = &config.database_url {
            Some(PgClient::connect(url, wipe_db).await?)
        } else {
            None
        };
        Ok(Self { config, rng, pg, journal, prev_hash: None })
    }

    /// Produce a demo payload for insertion into the journal and database.
    pub fn generate_payload(&mut self) -> (String, Value) {
        let user_id = self.rng.gen_range(1..=self.config.users);
        let container_id = format!("container_{}", self.rng.gen_range(1..=self.config.containers));
        let sentence: String = Sentence(1..3).fake();
        let author: String = Name().fake();
        let payload = json!({
            "user": user_id,
            "author": author,
            "content": sentence,
        });
        (container_id, payload)
    }

    pub async fn dispatch_payload(&mut self, timestamp: DateTime<Utc>) -> CJResult<()> {
        let (container_id, payload) = self.generate_payload();
        let result = self
            .journal
            .append_leaf(timestamp, self.prev_hash.map(PageContentHash::LeafHash), container_id.clone(), payload.clone())
            .await?;
        let leaf_hash = match result {
            PageContentHash::LeafHash(h) => h,
            _ => unreachable!(),
        };
        if let Some(pg) = &self.pg {
            let prev_hex = self.prev_hash.map(hex::encode).unwrap_or_else(|| "".to_string());
            pg.insert(&prev_hex, &hex::encode(leaf_hash), &payload).await?;
        }
        self.prev_hash = Some(leaf_hash);
        Ok(())
    }
}

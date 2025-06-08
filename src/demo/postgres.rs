#![cfg(feature = "demo")]

use crate::CJResult;
use tokio_postgres::{Client, NoTls};
use serde_json::Value;

pub struct PgClient {
    client: Client,
}

impl PgClient {
    pub async fn connect(url: &str, wipe: bool) -> CJResult<Self> {
        let (client, connection) = tokio_postgres::connect(url, NoTls).await.map_err(|e| crate::CJError::new(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                log::error!("Postgres connection error: {}", e);
            }
        });
        let pg = PgClient { client };
        pg.init_schema(wipe).await?;
        Ok(pg)
    }

    async fn init_schema(&self, wipe: bool) -> CJResult<()> {
        if wipe {
            self.client.batch_execute("DROP TABLE IF EXISTS demo_transactions;").await.map_err(|e| crate::CJError::new(e.to_string()))?;
        }
        self.client.batch_execute(
            "CREATE TABLE IF NOT EXISTS demo_transactions (\n                id SERIAL PRIMARY KEY,\n                prev_leaf_hash TEXT NOT NULL,\n                cj_leaf_hash TEXT NOT NULL UNIQUE,\n                payload JSONB NOT NULL,\n                created_at TIMESTAMPTZ DEFAULT NOW()\n            );",
        ).await.map_err(|e| crate::CJError::new(e.to_string()))?;
        Ok(())
    }

    pub async fn insert(&self, prev: &str, leaf: &str, payload: &Value) -> CJResult<()> {
        self.client
            .execute(
                "INSERT INTO demo_transactions (prev_leaf_hash, cj_leaf_hash, payload) VALUES ($1, $2, $3)",
                &[&prev, &leaf, &tokio_postgres::types::Json(payload)],
            )
            .await
            .map_err(|e| crate::CJError::new(e.to_string()))?;
        Ok(())
    }
}

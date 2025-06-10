#![cfg(feature = "demo")]
use crate::CJResult;
use log::{info, warn};
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;
use tokio_postgres::NoTls;
use std::path::PathBuf;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V15};
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::postgres::{PgEmbed, PgSettings};

pub enum PgHandle {
    Docker(String),
    Embedded(PgEmbed),
}

impl Drop for PgHandle {
    fn drop(&mut self) {
        match self {
            PgHandle::Docker(id) => {
                let _ = Command::new("docker").args(["rm", "-f", id]).output();
            }
            PgHandle::Embedded(pg) => {
                let _ = pg.stop_db_sync();
            }
        }
    }
}

pub async fn launch_postgres() -> CJResult<(String, PgHandle)> {
    if Command::new("docker").arg("--version").output().is_ok() {
        info!("Starting PostgreSQL using Docker");
        let output = Command::new("docker")
            .args([
                "run",
                "-d",
                "-e",
                "POSTGRES_DB=journal_demo",
                "-e",
                "POSTGRES_USER=demo",
                "-e",
                "POSTGRES_PASSWORD=demo",
                "-p",
                "5432:5432",
                "postgres:15",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let url = "postgres://demo:demo@localhost:5432/journal_demo";
                for _ in 0..10 {
                    if tokio_postgres::connect(url, NoTls).await.is_ok() {
                        return Ok((url.to_string(), PgHandle::Docker(id)));
                    }
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
        warn!("Docker not available or failed to start Postgres, falling back to embedded server");
    }
    info!("Starting embedded PostgreSQL");
    let settings = PgSettings {
        database_dir: PathBuf::from("target/pg_demo"),
        port: 5432,
        user: "demo".to_string(),
        password: "demo".to_string(),
        auth_method: PgAuthMethod::Plain,
        persistent: false,
        timeout: Some(Duration::from_secs(15)),
        migration_dir: None,
    };
    let fetch = PgFetchSettings { version: PG_V15, ..Default::default() };
    let mut pg = PgEmbed::new(settings, fetch)
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.setup().await.map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.start_db().await.map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.create_database("journal_demo")
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    let url = pg.full_db_uri("journal_demo");
    Ok((url, PgHandle::Embedded(pg)))
}


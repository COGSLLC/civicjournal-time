#![cfg(feature = "demo")]
use crate::CJResult;
use log::{info, warn};
use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V15};
use pg_embed::postgres::{PgEmbed, PgSettings};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tokio::time::sleep;
use tokio_postgres::NoTls;

pub enum PgHandle {
    Docker(String),
    Embedded(PgEmbed),
}

fn get_available_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("failed to bind random port")
        .local_addr()
        .unwrap()
        .port()
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
    let port = get_available_port();
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
                &format!("{}:5432", port),
                "postgres:15",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let url = format!("postgres://demo:demo@localhost:{}/journal_demo", port);
                for _ in 0..10 {
                    if tokio_postgres::connect(&url, NoTls).await.is_ok() {
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
        port,
        user: "demo".to_string(),
        password: "demo".to_string(),
        auth_method: PgAuthMethod::Plain,
        persistent: false,
        timeout: Some(Duration::from_secs(15)),
        migration_dir: None,
    };
    let fetch = PgFetchSettings {
        version: PG_V15,
        ..Default::default()
    };
    let mut pg = PgEmbed::new(settings, fetch)
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.setup()
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.start_db()
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    pg.create_database("journal_demo")
        .await
        .map_err(|e| crate::CJError::new(e.to_string()))?;
    let url = format!("postgres://demo:demo@localhost:{}/journal_demo", port);
    Ok((url, PgHandle::Embedded(pg)))
}

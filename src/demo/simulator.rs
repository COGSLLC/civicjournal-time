#![cfg(feature = "demo")]

use crate::demo::DemoConfig;
use crate::demo::generator::LeafGenerator;
use crate::CJResult;
use crate::api::async_api::Journal;
use chrono::{DateTime, Utc, TimeZone, Duration};
use chrono::NaiveDate;
use tokio::time::{sleep, Duration as TokioDuration};

/// Drives time forward and invokes the generator to create leaves.
pub struct Simulator {
    config: DemoConfig,
    generator: LeafGenerator,
    live: bool,
}

impl Simulator {
    /// Create a new Simulator with the given configuration.
    pub async fn new(config: DemoConfig, journal: Journal, wipe_db: bool, live: bool) -> CJResult<Self> {
        let generator = LeafGenerator::new(config.clone(), journal, wipe_db).await?;
        Ok(Self { config, generator, live })
    }

    /// Run the simulation. This is a placeholder implementation.
    pub async fn run(&mut self) -> CJResult<()> {
        let mut current = to_datetime(self.config.start_date);
        let end = to_datetime(self.config.end_date);

        while current <= end {
            for _ in 0..self.config.leaf_rate.per_real_second {
                self.generator.dispatch_payload(current).await?;
            }
            current = current + Duration::days(30);
            if self.live {
                sleep(TokioDuration::from_secs_f32(self.config.rate.real_seconds_per_month)).await;
            }
        }
        Ok(())
    }
}

fn to_datetime(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
}

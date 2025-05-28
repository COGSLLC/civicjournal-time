use std::sync::Arc;
use std::any::Any;
use thiserror::Error;

// Conditionally use appropriate Mutex based on feature
#[cfg(feature = "web")]
use tokio::sync::Mutex;
#[cfg(not(feature = "web"))]
use std::sync::Mutex;

use crate::rate_limiter::{RateLimiter, RateLimitSettings, RateLimitError};
use crate::schema::{CivicObject, Delta, StateValidator, DefaultValidator};
#[cfg(feature = "web")]
use crate::schema::AsyncStateValidator;
use crate::recovery::{Journal as RecoveryJournal, RecoveryError};

#[derive(Debug, Error)]
pub enum JournalApiError {
    #[error("Rate limited: {0}")]
    RateLimited(String),
    
    #[error("Temporarily banned")]
    TemporarilyBanned,
    
    #[error("Validation error: {0}")]
    ValidationError(String),
    
    #[error("Recovery error: {0}")]
    RecoveryError(#[from] RecoveryError),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Inner state shared between sync and async implementations
struct JournalApiInner {
    journal: RecoveryJournal,
    rate_limiter: RateLimiter,
    validator: Arc<dyn StateValidator>,
}

/// Main API for interacting with the CivicJournal
pub struct JournalApi {
    inner: Arc<Mutex<JournalApiInner>>,
}

impl JournalApi {
    /// Create a new JournalApi with default settings
    pub fn new() -> Self {
        let journal = RecoveryJournal::new();
        
        // Configure rate limits based on security needs
        let rate_limiter = RateLimiter::new(RateLimitSettings {
            max_requests: 500,         // 500 requests per minute
            time_window: std::time::Duration::from_secs(60),
            max_burst: 20,             // 20 concurrent requests
            ban_duration: std::time::Duration::from_secs(300), // 5 minute ban
            violations_before_ban: 3,  // 3 strikes
            cleanup_interval: std::time::Duration::from_secs(60),
        });
        
        let validator = Arc::new(DefaultValidator::new());
        
        Self {
            inner: Arc::new(Mutex::new(JournalApiInner {
                journal,
                rate_limiter,
                validator,
            })),
        }
    }
    
    /// Append an entry to the journal with rate limiting and validation (async version)
    #[cfg(feature = "web")]
    pub async fn append_entry(
        &self,
        client_id: &str,
        entry: CivicObject,
    ) -> Result<(), JournalApiError> {
        // Get a clone of the inner state
        let inner = self.inner.clone();
        
        // Spawn a blocking task to handle the operation
        tokio::task::spawn_blocking(move || {
            let mut inner = inner.lock().unwrap();
            
            // Check rate limit (sync version in blocking context)
            inner.rate_limiter.check_sync(client_id)?;
            
            // Get current state for object
            let current_state = inner.journal.get_object(entry.id().to_string().as_str());
            
            // Create delta
            let delta = Delta::new(
                crate::schema::Operation::Create { object: entry.clone() },
                chrono::Utc::now(),
                entry.hash(),
            );
            
            // Validate delta
            if let Err(e) = inner.validator.validate_delta_sync(current_state.as_ref(), &delta) {
                return Err(JournalApiError::ValidationError(e.to_string()));
            }
            
            // Add delta to journal
            inner.journal.add_delta(entry.id().to_string(), delta)?;
            
            Ok::<_, JournalApiError>(())
        }).await??; // Unwrap the JoinError and the Result
        
        Ok(())
    }
    
    /// Append an entry to the journal with rate limiting and validation (sync version)
    #[cfg(not(feature = "web"))]
    pub fn append_entry(
        &self,
        client_id: &str,
        entry: CivicObject,
    ) -> Result<(), JournalApiError> {
        let mut inner = self.inner.lock().unwrap();
        
        // Check rate limit (sync version)
        inner.rate_limiter.check_sync(client_id).map_err(|e| match e {
            RateLimitError::LimitExceeded(max, window) => {
                JournalApiError::RateLimited(format!(
                    "Rate limit exceeded: {} requests allowed per {:?}",
                    max, window
                ))
            }
            RateLimitError::BurstExceeded(max) => {
                JournalApiError::RateLimited(format!(
                    "Too many concurrent requests: maximum {}", max
                ))
            }
            RateLimitError::TemporaryBan => {
                JournalApiError::TemporarilyBanned
            }
        })?;
        
        // Get current state for object
        let current_state = inner.journal.get_object(entry.id().to_string().as_str());
        
        // Create delta
        let delta = Delta::new(
            crate::schema::Operation::Create { object: entry.clone() },
            chrono::Utc::now(),
            entry.hash(),
        );
        
        // Validate delta
        inner.validator.validate_delta_sync(current_state.as_ref(), &delta)
            .map_err(|e| JournalApiError::ValidationError(e.to_string()))?;
        
        // Add delta to journal
        inner.journal.add_delta(entry.id().to_string(), delta)?;
        
        Ok(())
    }
    
    /// Recover the journal from corruption (async version)
    #[cfg(feature = "web")]
    pub async fn recover(&self) -> Result<String, JournalApiError> {
        let inner = self.inner.clone();
        
        // Spawn a blocking task to handle the operation
        let result = tokio::task::spawn_blocking(move || {
            let mut inner = inner.lock().unwrap();
            inner.journal.recover()
        }).await;
        
        // Handle the result of the spawned task
        let report = match result {
            Ok(Ok(report)) => report,
            Ok(Err(e)) => return Err(JournalApiError::RecoveryError(e)),
            Err(e) => return Err(JournalApiError::RecoveryError(RecoveryError::Other(e.to_string()))),
        };
        
        // Format report into human-readable message
        let message = format!(
            "Recovery completed: {} entries verified, {} entries recovered, {} corrupted entries found. Merkle tree rebuilt: {}",
            report.verified_entries,
            report.recovered_entries,
            report.corrupt_entries.len(),
            report.rebuilt_merkle_tree
        );
        
        Ok(message)
    }
    
    /// Recover the journal from corruption (sync version)
    #[cfg(not(feature = "web"))]
    pub fn recover(&self) -> Result<String, JournalApiError> {
        // In the sync version, we'll just return a message indicating that recovery
        // needs to be performed through the async API when the web feature is enabled
        Err(JournalApiError::RecoveryError(
            RecoveryError::StateReconstruction(
                "Recovery is only available through the async API when the web feature is enabled".to_string()
            )
        ))
    }
    
    /// Detect forks in the journal (async version)
    #[cfg(feature = "web")]
    pub async fn detect_forks(&self) -> Result<Vec<String>, JournalApiError> {
        let inner = self.inner.clone();
        
        // Spawn a blocking task to handle the operation
        let forks = tokio::task::spawn_blocking(move || {
            let inner = inner.lock().unwrap();
            inner.journal.detect_forks()
        }).await
        .map_err(|e| JournalApiError::RecoveryError(RecoveryError::Other(e.to_string())))?;
        
        // Format forks into human-readable messages
        let messages = forks.into_iter().map(|fork| {
            format!(
                "Fork detected at point {:?} with {} branches",
                fork.fork_point,
                fork.branches.len()
            )
        }).collect();
        
        Ok(messages)
    }
    
    /// Detect forks in the journal (sync version)
    #[cfg(not(feature = "web"))]
    pub fn detect_forks(&self) -> Result<Vec<String>, JournalApiError> {
        let inner = self.inner.lock().unwrap();
        
        // Call the detect_forks method directly (synchronous version)
        let forks = inner.journal.detect_forks();
        
        // Format forks into human-readable messages
        let messages = forks.into_iter().map(|fork| {
            format!(
                "Fork detected at point {:?} with {} branches",
                fork.fork_point,
                fork.branches.len()
            )
        }).collect();
        
        Ok(messages)
    }
}

impl Default for JournalApi {
    fn default() -> Self {
        Self::new()
    }
}

// Extension trait to add rate limiting to the existing Journal
pub trait RateLimitedJournal {
    fn with_rate_limit(self, settings: RateLimitSettings) -> JournalApi;
}

// Extension trait to add validation to the existing Journal
pub trait ValidatedJournal {
    fn with_validator<V: StateValidator + 'static>(self, validator: V) -> Self;
}

impl RateLimitedJournal for JournalApi {
    fn with_rate_limit(mut self, settings: RateLimitSettings) -> Self {
        let mut inner = self.inner.lock().unwrap();
        inner.rate_limiter = RateLimiter::new(settings);
        drop(inner); // Explicitly drop the lock guard
        self
    }
}

impl ValidatedJournal for JournalApi {
    fn with_validator<V: StateValidator + 'static>(mut self, validator: V) -> Self {
        let mut inner = self.inner.lock().unwrap();
        inner.validator = Arc::new(validator);
        drop(inner); // Explicitly drop the lock guard
        self
    }
}

// Add extension methods to recovery::Journal
impl RecoveryJournal {
    /// Get an object from the journal by ID
    pub fn get_object(&self, id: &str) -> Option<CivicObject> {
        self.objects.get(id).cloned()
    }
    
    /// Add a delta to the journal
    pub fn add_delta(&mut self, id: String, delta: Delta) -> Result<(), RecoveryError> {
        // Create a journal entry from the delta
        let entry = crate::recovery::JournalEntry {
            sequence: self.entries.len() as u64,
            timestamp: delta.timestamp,
            object_id: id.clone(),
            delta_hash: [0u8; 32], // This will be calculated
            previous_hash: delta.previous_hash,
            signature: vec![1, 2, 3], // Mock signature for now
        };
        
        // Add the entry to the journal
        self.entries.push(entry);
        
        // Update object state based on delta
        match &delta.operation {
            crate::schema::Operation::Create { object } => {
                self.objects.insert(id, object.clone());
            },
            // Handle other operations here
            _ => {}
        }
        
        Ok(())
    }
}

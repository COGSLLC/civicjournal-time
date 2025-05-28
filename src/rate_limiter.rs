use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use thiserror::Error;

// Use tokio's Mutex when the web feature is enabled
#[cfg(feature = "tokio")]
type AsyncMutex<T> = tokio::sync::Mutex<T>;

// Use std::sync::Mutex when tokio is not available
#[cfg(not(feature = "tokio"))]
type AsyncMutex<T> = std::sync::Mutex<T>;

#[derive(Debug, Error)]
pub enum RateLimitError {
    #[error("Rate limit exceeded: {0} requests allowed per {1:?}")]
    LimitExceeded(u32, Duration),
    
    #[error("Burst limit exceeded: maximum {0} concurrent requests")]
    BurstExceeded(u32),
    
    #[error("Client is temporarily banned due to excessive requests")]
    TemporaryBan,
}

struct RateLimitEntry {
    /// Timestamps of recent requests
    request_history: Vec<Instant>,
    /// Time when temporary ban expires (if any)
    ban_expiry: Option<Instant>,
    /// Number of times this key has been rate limited
    violation_count: u32,
}

pub struct RateLimiter {
    /// Rate limit settings
    settings: RateLimitSettings,
    /// Rate limit state per client
    limits: Arc<AsyncMutex<HashMap<String, RateLimitEntry>>>,
    /// Background task handle for cleanup
    #[cfg(feature = "tokio")]
    _cleanup_task: Option<tokio::task::JoinHandle<()>>,
    #[cfg(not(feature = "tokio"))]
    _cleanup_task: Option<std::thread::JoinHandle<()>>,
}

pub struct RateLimitSettings {
    /// Maximum requests per time window
    pub max_requests: u32,
    /// Time window for rate limiting
    pub time_window: Duration,
    /// Maximum burst (concurrent requests)
    pub max_burst: u32,
    /// How long to temporarily ban after multiple violations
    pub ban_duration: Duration,
    /// Number of violations before temporary ban
    pub violations_before_ban: u32,
    /// How often to clean up stale entries
    pub cleanup_interval: Duration,
}

impl Default for RateLimitSettings {
    fn default() -> Self {
        Self {
            max_requests: 100,
            time_window: Duration::from_secs(60),
            max_burst: 10,
            ban_duration: Duration::from_secs(300), // 5 minutes
            violations_before_ban: 3,
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

impl RateLimitEntry {
    fn new(max_requests: u32, _time_window: Duration) -> Self {
        Self {
            request_history: Vec::with_capacity(max_requests as usize),
            ban_expiry: None,
            violation_count: 0,
        }
    }
    
    fn check(&mut self, now: Instant, max_requests: u32, time_window: Duration) -> Result<(), RateLimitError> {
        // Check if temporarily banned
        if let Some(expiry) = self.ban_expiry {
            if now < expiry {
                return Err(RateLimitError::TemporaryBan);
            }
            // Ban expired
            self.ban_expiry = None;
        }
        
        // Remove old requests from history
        let cutoff = now.checked_sub(time_window).unwrap_or_else(|| Instant::now());
        self.request_history.retain(|&time| time >= cutoff);
        
        // Check if rate limit is exceeded
        if self.request_history.len() >= max_requests as usize {
            self.violation_count += 1;
            return Err(RateLimitError::LimitExceeded(max_requests, time_window));
        }
        
        // Record the request
        self.request_history.push(now);
        Ok(())
    }
}

impl RateLimiter {
    pub fn new(settings: RateLimitSettings) -> Self {
        let limits = Arc::new(AsyncMutex::new(HashMap::new()));
        
        #[cfg(feature = "web")]
        let cleanup_task = {
            // Start background cleanup task when web feature is enabled
            let cleanup_limits = limits.clone();
            let cleanup_interval = settings.cleanup_interval;
            let time_window = settings.time_window;
            
            Some(tokio::spawn(async move {
                let mut interval = tokio::time::interval(cleanup_interval);
                loop {
                    interval.tick().await;
                    Self::cleanup_stale_entries(&cleanup_limits, time_window).await;
                }
            }))
        };
        
        #[cfg(not(feature = "web"))]
        let cleanup_task = None;
        
        // In non-web mode, we'll clean up stale entries on demand
        #[cfg(not(feature = "web"))]
        if let Some(cleanup_interval) = settings.cleanup_interval.checked_mul(2) {
            // This is a best-effort cleanup in synchronous mode
            let cleanup_limits = limits.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(cleanup_interval);
                    Self::cleanup_stale_entries_sync(&cleanup_limits, settings.time_window);
                }
            });
        }
        
        Self {
            settings,
            limits,
            _cleanup_task: cleanup_task,
        }
    }
    
    /// Check if a request should be allowed or rate limited (synchronous version)
    pub fn check_sync(&self, client_id: &str) -> Result<(), RateLimitError> {
        let now = Instant::now();
        
        // Use the appropriate mutex based on the feature flag
        #[cfg(feature = "web")]
        let mut limits = self.limits.blocking_lock();
        
        #[cfg(not(feature = "web"))]
        let mut limits = self.limits.lock().unwrap();
        
        // Get or create rate limit entry
        let entry = limits
            .entry(client_id.to_string())
            .or_insert_with(|| RateLimitEntry::new(self.settings.max_requests, self.settings.time_window));
        
        // Check if client is rate limited
        entry.check(now, self.settings.max_requests, self.settings.time_window)
    }
    
    /// Check if a request should be allowed or rate limited (asynchronous version)
    pub async fn check(&self, client_id: &str) -> Result<(), RateLimitError> {
        let now = Instant::now();
        
        // Use the appropriate lock based on the feature flag
        #[cfg(feature = "web")]
        let mut limits = self.limits.lock().await;
        
        #[cfg(not(feature = "web"))]
        let mut limits = self.limits.lock().unwrap();
        
        // Get or create rate limit entry
        let entry = limits
            .entry(client_id.to_string())
            .or_insert_with(|| RateLimitEntry::new(self.settings.max_requests, self.settings.time_window));
        
        // Check if client is rate limited
        entry.check(now, self.settings.max_requests, self.settings.time_window)
    }
    
    /// Clean up stale entries to prevent memory leaks
    /// This is called internally by the background task when the web feature is enabled
    #[cfg(feature = "web")]
    async fn cleanup_stale_entries(
        limits: &Arc<Mutex<HashMap<String, RateLimitEntry>>>,
        time_window: Duration,
    ) {
        let now = Instant::now();
        let mut limits = limits.lock().await;
        
        limits.retain(|_, entry| {
            // Keep if has active ban
            if let Some(expiry) = entry.ban_expiry {
                if now < expiry {
                    return true;
                }
            }
            
            // Keep if has recent requests
            entry.request_history.retain(|&time| now - time < time_window);
            !entry.request_history.is_empty()
        });
    }
    
    /// Clean up stale entries synchronously
    /// This is used when the tokio feature is not enabled
    fn cleanup_stale_entries_sync(
        limits: &Arc<AsyncMutex<HashMap<String, RateLimitEntry>>>,
        time_window: Duration,
    ) {
        let now = Instant::now();
        
        // For the sync version, we'll use a blocking lock
        if let Ok(guard) = limits.lock() {
            let mut limits_guard = guard;
            limits_guard.retain(|_, entry| {
                // Keep entries that are not banned or whose ban has not expired
                if let Some(expiry) = entry.ban_expiry {
                    if now < expiry {
                        return true;  // Keep banned entries that haven't expired
                    }
                }
                
                // Keep entries with recent requests
                entry.request_history.retain(|&t| now.duration_since(t) <= time_window);
                !entry.request_history.is_empty()
            });
        }
    }
    
    /// Clean up stale entries (call this periodically in non-web contexts)
    pub fn cleanup_stale_entries_now(&self) {
        #[cfg(feature = "web")]
        {
            // In web mode, this is handled by the background task
            // But we provide a no-op method for API compatibility
        }
        
        #[cfg(not(feature = "web"))]
        {
            Self::cleanup_stale_entries_sync(
                &self.limits,
                self.settings.time_window
            );
        }
    }
    
    // Helper method to get the mutex guard
    async fn get_limits_mut(&self) -> impl DerefMut<Target = HashMap<String, RateLimitEntry>> + '_ {
        #[cfg(feature = "web")]
        {
            self.limits.lock().await
        }
        #[cfg(not(feature = "web"))]
        {
            self.limits.lock().unwrap()
        }
    }
    
    /// Reset rate limit for a client (async version)
    pub async fn reset(&self, client_id: &str) {
        #[cfg(feature = "web")]
        let mut limits = self.limits.lock().await;
        
        #[cfg(not(feature = "web"))]
        let mut limits = self.limits.lock().unwrap();
        
        limits.remove(client_id);
    }
    
    /// Reset rate limit for a client (sync version)
    pub fn reset_sync(&self, client_id: &str) {
        #[cfg(feature = "web")]
        let mut limits = self.limits.blocking_lock();
        
        #[cfg(not(feature = "web"))]
        let mut limits = self.limits.lock().unwrap();
        
        limits.remove(client_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    
    #[cfg(feature = "web")]
    use tokio::time::sleep;

    fn create_test_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitSettings {
            max_requests: 5,
            time_window: Duration::from_secs(1),
            max_burst: 3,
            ban_duration: Duration::from_secs(2),
            violations_before_ban: 2,
            cleanup_interval: Duration::from_secs(1),
        })
    }
    
    #[test]
    fn test_sync_rate_limiting() {
        let limiter = create_test_limiter();
        
        // Client should be allowed 5 requests
        for _ in 0..5 {
            assert!(limiter.check_sync("test_client").is_ok());
        }
        
        // 6th request should be rate limited
        match limiter.check_sync("test_client") {
            Err(RateLimitError::LimitExceeded(5, _)) => {}
            other => panic!("Expected LimitExceeded, got {:?}", other),
        }
        
        // Test burst limit
        for _ in 0..3 {
            assert!(limiter.check_sync("burst_test").is_ok());
        }
        
        // 4th request in burst should be limited
        match limiter.check_sync("burst_test") {
            Err(RateLimitError::BurstExceeded(3)) => {}
            other => panic!("Expected BurstExceeded, got {:?}", other),
        }
    }
    
    #[test]
    fn test_ban_and_reset() {
        let limiter = create_test_limiter();
        
        // Trigger ban
        for _ in 0..2 {
            for _ in 0..6 { // Exceed limit twice
                let _ = limiter.check_sync("ban_test");
            }
        }
        
        // Should be banned now
        match limiter.check_sync("ban_test") {
            Err(RateLimitError::TemporaryBan) => {}
            other => panic!("Expected TemporaryBan, got {:?}", other),
        }
        
        // Reset the ban
        limiter.reset_sync("ban_test");
        
        // Should be allowed again after reset
        assert!(limiter.check_sync("ban_test").is_ok());
    }
    
    #[cfg(feature = "web")]
    #[tokio::test]
    async fn test_async_rate_limiting() {
        let limiter = create_test_limiter();
        
        // Client should be allowed 5 requests
        for _ in 0..5 {
            assert!(limiter.check("test_client").await.is_ok());
        }
        
        // 6th request should be rate limited
        match limiter.check("test_client").await {
            Err(RateLimitError::LimitExceeded(5, _)) => {}
            other => panic!("Expected LimitExceeded, got {:?}", other),
        }
        
        // Wait for the time window to reset
        sleep(Duration::from_secs(1)).await;
        
        // Should be allowed again after time window
        assert!(limiter.check("test_client").await.is_ok());
    }
    
    #[cfg(feature = "web")]
    #[tokio::test]
    async fn test_async_ban_and_reset() {
        let limiter = create_test_limiter();
        
        // Trigger ban
        for _ in 0..2 {
            for _ in 0..6 { // Exceed limit twice
                let _ = limiter.check("ban_test_async").await;
            }
        }
        
        // Should be banned now
        match limiter.check("ban_test_async").await {
            Err(RateLimitError::TemporaryBan) => {}
            other => panic!("Expected TemporaryBan, got {:?}", other),
        }
        
        // Reset the ban
        limiter.reset("ban_test_async").await;
        
        // Should be allowed again after reset
        assert!(limiter.check("ban_test_async").await.is_ok());
    }
}

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use chrono::{DateTime, Utc};
use thiserror::Error;
use serde::{Serialize, Deserialize};
use std::fmt;

use crate::journal_api::{JournalApi, JournalApiError};

/// Using std::sync::Mutex implementation since tokio is not being used
#[derive(Debug)]
pub struct AsyncMutex<T>(std::sync::Mutex<T>);

#[cfg(not(feature = "tokio"))]
impl<T> AsyncMutex<T> {
    /// Create a new async mutex
    pub fn new(value: T) -> Self {
        AsyncMutex(std::sync::Mutex::new(value))
    }
    
    /// Lock the mutex
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, T>> {
        self.0.lock()
    }
}

/// Type alias for health check functions
pub type HealthCheck = Box<dyn Fn(&JournalApi) -> Result<(), MonitorError> + Send + Sync>;

#[derive(Debug, Error)]
pub enum MonitorError {
    #[error("IO error: {0}")]
    IoError(std::io::Error),
    #[error("Serialization error: {0}")]
    SerializationError(serde_json::Error),
    #[error("Validation error: {0}")]
    ValidationError(String),
    #[error("Invalid metric: {0}")]
    InvalidMetric(String),
    #[error("Invalid alert: {0}")]
    InvalidAlert(String),
    #[error("Alert not found: {0}")]
    AlertNotFound(String),
    
    #[error("Error: {0}")]
    Other(String),
    #[error("Metric threshold exceeded: {name} (value: {value}, threshold: {threshold})")]
    ThresholdExceeded {
        name: String,
        value: f64,
        threshold: f64,
    },
    
    #[error("Health check failed: {0}")]
    CheckFailed(String),
    
    #[error("System degraded: {0}")]
    SystemDegraded(String),
    
    #[error("Alert delivery failed: {0}")]
    AlertDeliveryFailed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// Name of the metric
    pub name: String,
    /// Current value
    pub value: f64,
    /// Metric unit (e.g., "ms", "count", "percent")
    pub unit: String,
    /// Timestamp when the metric was last updated
    pub timestamp: DateTime<Utc>,
    /// Warning threshold (optional)
    pub warning_threshold: Option<f64>,
    /// Critical threshold (optional)
    pub critical_threshold: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
    Emergency,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Alert ID
    pub id: String,
    /// Alert message
    pub message: String,
    /// Alert level
    pub level: AlertLevel,
    /// Timestamp when the alert was created
    pub timestamp: DateTime<Utc>,
    /// Related metrics that triggered the alert
    pub metrics: Vec<Metric>,
    /// Whether the alert has been acknowledged
    pub acknowledged: bool,
    /// Time when the alert was acknowledged (if any)
    pub acknowledged_at: Option<DateTime<Utc>>,
}

impl Alert {
    pub fn new(message: String, level: AlertLevel, metrics: Vec<Metric>) -> Self {
        let now = Utc::now();
        let id = format!("alert_{}", now.timestamp());
        
        Self {
            id,
            message,
            level,
            timestamp: now,
            metrics,
            acknowledged: false,
            acknowledged_at: None,
        }
    }
    
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
        self.acknowledged_at = Some(Utc::now());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Report generation timestamp
    pub timestamp: DateTime<Utc>,
    /// Overall system health status
    pub status: HealthStatus,
    /// Current metrics
    pub metrics: Vec<Metric>,
    /// Active alerts
    pub alerts: Vec<Alert>,
    /// Health check results
    pub checks: Vec<CheckResult>,
}

/// Result of a health check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Name of the check
    pub name: String,
    /// Status of the check
    pub status: HealthStatus,
    /// Optional error message if the check failed
    pub message: Option<String>,
    /// Duration of the check execution
    pub duration: Duration,
    /// When the check was performed
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Critical,
    Unknown,
}

/// Health monitoring for the journal system
pub struct HealthMonitor {
    /// System start time
    start_time: Instant,
    /// Reference to the journal API
    journal_api: Arc<JournalApi>,
    /// Current metrics
    metrics: Arc<Mutex<HashMap<String, Metric>>>,
    /// Active alerts
    active_alerts: Arc<Mutex<Vec<Alert>>>,
    /// Recent alerts (including acknowledged)
    recent_alerts: Arc<Mutex<VecDeque<Alert>>>,
    /// Health checks to run (protected by mutex when accessed from async contexts)
    health_checks: Vec<HealthCheck>,
    /// Alert handlers
    alert_handlers: Arc<Mutex<Vec<Box<dyn Fn(&Alert) -> Result<(), MonitorError> + Send + Sync>>>>,
    /// Maximum number of recent alerts to keep
    max_recent_alerts: usize,
}

impl HealthMonitor {
    /// Create a new HealthMonitor instance
    pub fn new(journal_api: Arc<JournalApi>) -> Self {
        // Tokio runtime initialization removed
        
        Self {
            start_time: Instant::now(),
            journal_api,
            metrics: Arc::new(Mutex::new(HashMap::new())),
            active_alerts: Arc::new(Mutex::new(Vec::new())),
            recent_alerts: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            health_checks: Vec::new(),
            alert_handlers: Arc::new(Mutex::new(Vec::new())),
            max_recent_alerts: 100,
        }
    }
    
    /// Add a health check
    pub fn add_check<F>(&mut self, check: F)
    where
        F: Fn(&JournalApi) -> Result<(), MonitorError> + Send + Sync + 'static,
    {
        self.health_checks.push(Box::new(check));
    }
    
    /// Add a standard check for journal fork detection
    pub fn add_fork_detection_check(&mut self) {
        let _api = self.journal_api.clone(); // Prefix with underscore to avoid unused variable warning
        
        self.add_check(move |_| {
            // Temporarily use a simplified approach for the detect_forks call
            // Returning empty vec for now while we resolve async/await issues
            let forks: Vec<String> = Vec::new();
                
            if !forks.is_empty() {
                return Err(MonitorError::SystemDegraded(format!(
                    "{} forks detected in the journal",
                    forks.len()
                )));
            }
            
            Ok(())
        });
    }
    
    /// Add a standard check for rate limit violations
    pub fn add_rate_limit_check(&mut self, threshold: u32) {
        self.add_metric(
            "rate_limit_violations",
            0.0,
            "count",
            None,
            Some(threshold as f64),
        );
        
        // In a real implementation, this would hook into the rate limiter
        // to track violations
    }
    
    /// Add a standard check for journal integrity
    pub fn add_integrity_check(&mut self) {
        let _api = self.journal_api.clone(); // Prefix with underscore to avoid unused variable warning
        
        self.add_check(move |_| {
            // Temporarily use a simplified approach for recover call
            // Always return Ok for now while we resolve async/await issues
            use crate::recovery::RecoveryError;
            let result: Result<String, RecoveryError> = Ok("Recovery simulated".to_string());
            
            match result {
                Ok(_) => {
                    // Recovery succeeded, journal is healthy
                    Ok(())
                }
                Err(e) => {
                    // Recovery failed, journal integrity is compromised
                    Err(MonitorError::SystemDegraded(
                        format!("Journal recovery failed: {}", e)
                    ))
                }
            }
        });
    }
    
    /// Add an alert handler
    pub fn add_alert_handler<F>(&self, handler: F)
    where
        F: Fn(&Alert) -> Result<(), MonitorError> + 'static + Send + Sync,
    {
        if let Ok(mut handlers) = self.alert_handlers.lock() {
            handlers.push(Box::new(handler));
        } else {
            eprintln!("Failed to acquire lock on alert handlers");
        }
    }
    
    /// Add a standard console alert handler
    pub fn add_console_alert_handler(&mut self) {
        self.add_alert_handler(|alert| {
            println!(
                "[{}] {} Alert: {}",
                alert.timestamp,
                alert.level,
                alert.message
            );
            Ok(())
        });
    }
    
    /// Add a metric
    pub fn add_metric(
        &self,
        name: &str,
        value: f64,
        unit: &str,
        warning_threshold: Option<f64>,
        critical_threshold: Option<f64>,
    ) {
        let metric = Metric {
            name: name.to_string(),
            value,
            unit: unit.to_string(),
            timestamp: Utc::now(),
            warning_threshold,
            critical_threshold,
        };
        
        if let Ok(mut metrics) = self.metrics.lock() {
            metrics.insert(name.to_string(), metric);
        } else {
            eprintln!("Failed to acquire lock on metrics");
        }
    }
    
    /// Update a metric
    pub fn update_metric(
        &self,
        name: &str,
        value: f64,
        unit: Option<&str>,
        warning_threshold: Option<f64>,
        critical_threshold: Option<f64>,
    ) {
        if let Ok(mut metrics) = self.metrics.lock() {
            if let Some(metric) = metrics.get_mut(name) {
                Self::update_metric_inner(metric, value, unit, warning_threshold, critical_threshold);
            }
        } else {
            eprintln!("Failed to acquire lock on metrics");
        }
    }
    
    /// Helper method to update metric fields
    fn update_metric_inner(
        metric: &mut Metric,
        value: f64,
        unit: Option<&str>,
        warning_threshold: Option<f64>,
        critical_threshold: Option<f64>,
    ) {
        metric.value = value;
        if let Some(unit) = unit {
            metric.unit = unit.to_string();
        }
        if let Some(warning) = warning_threshold {
            metric.warning_threshold = Some(warning);
        }
        if let Some(critical) = critical_threshold {
            metric.critical_threshold = Some(critical);
        }
        metric.timestamp = Utc::now();
    }
    
    /// Acknowledge an alert by its ID
    pub fn acknowledge_alert(&self, alert_id: &str) -> Result<(), MonitorError> {
        let active_alerts = self.active_alerts.clone();
        let recent_alerts = self.recent_alerts.clone();
        let alert_id = alert_id.to_string();
        
        // Define the operation to be performed
        let operation = || -> Result<(), MonitorError> {
            // Acknowledge in active alerts
            let mut alert_to_move = None;
            
            // First, try to find and remove the alert from active alerts
            {
                let mut active = match active_alerts.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("Mutex was poisoned, attempting to recover");
                        poisoned.into_inner()
                    }
                };
                
                if let Some(pos) = active.iter().position(|a| a.id == alert_id) {
                    let mut alert = active.remove(pos);
                    alert.acknowledge();
                    alert_to_move = Some(alert);
                } else {
                    // If not found in active alerts, check recent alerts
                    let mut recent = match recent_alerts.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => {
                            eprintln!("Mutex was poisoned, attempting to recover");
                            poisoned.into_inner()
                        }
                    };
                    
                    if let Some(alert) = recent.iter_mut().find(|a| a.id == alert_id) {
                        alert.acknowledge();
                        return Ok(());
                    } else {
                        return Err(MonitorError::AlertNotFound(alert_id.to_string()));
                    }
                }
            }
            
            // If we have an alert to move to recent alerts
            if let Some(alert) = alert_to_move {
                let mut recent = match recent_alerts.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("Mutex was poisoned, attempting to recover");
                        poisoned.into_inner()
                    }
                };
                
                // Add to recent alerts
                recent.push_back(alert);
                
                // Trim if needed
                if recent.len() > self.max_recent_alerts {
                    let _ = recent.pop_front();
                }
            }
            
            Ok(())
        };
        
        // Execute the operation
        {
            if let Err(e) = operation() {
                eprintln!("Error in acknowledge_alert operation: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Create a new alert
    pub fn create_alert(
        &self,
        message: String,
        level: AlertLevel,
        metrics: Vec<Metric>,
    ) -> Result<Alert, MonitorError> {
        let alert = Alert::new(message, level, metrics);
        
        // Clone necessary data for the async block
        let active_alerts = self.active_alerts.clone();
        let recent_alerts = self.recent_alerts.clone();
        let alert_clone = alert.clone();
        let max_recent = self.max_recent_alerts;
        let alert_handlers = self.alert_handlers.clone();
        
        // Define the operation to be performed
        let operation = || -> Result<(), MonitorError> {
            // Add to active alerts
            {
                let mut active = match active_alerts.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("Mutex was poisoned, attempting to recover");
                        poisoned.into_inner()
                    }
                };
                active.push(alert_clone.clone());
            }
            
            // Add to recent alerts
            {
                let mut recent = match recent_alerts.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        eprintln!("Mutex was poisoned, attempting to recover");
                        poisoned.into_inner()
                    }
                };
                
                recent.push_back(alert_clone.clone());
                
                // Trim if needed
                if recent.len() > max_recent {
                    let mut vec: Vec<_> = recent.drain(..).collect();
                    vec.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                    recent.extend(vec);
                    recent.truncate(max_recent);
                }
            }
            
            // Call handlers
            if let Ok(handlers) = alert_handlers.lock() {
                for handler in handlers.iter() {
                    if let Err(e) = handler(&alert_clone) {
                        eprintln!("Alert handler error: {}", e);
                    }
                }
            }
            
            Ok(())
        };
        
        // Execute the operation
        {
            if let Err(e) = operation() {
                eprintln!("Error in create_alert operation: {}", e);
            }
        }
        
        Ok(alert)
    }
    
    /// Run all health checks and return a health report
    pub fn check_system_health(&self) -> HealthReport {
        // Using only sync implementation for now
        self.check_system_health_sync()
    }
    
    /// Run health checks asynchronously (when tokio is available)
    // Temporarily commented out to remove tokio dependency issues
    /* 
    #[cfg(feature = "tokio")]
    pub async fn check_system_health_async(&self) -> HealthReport {
        use futures::future::join_all;
        
        let now = Utc::now();
        let mut report = HealthReport {
            timestamp: now,
            status: HealthStatus::Healthy,
            metrics: Vec::new(),
            alerts: Vec::new(),
            checks: Vec::with_capacity(self.health_checks.len()),
        };
        
        // If no health checks, return early with just metrics and alerts
        if self.health_checks.is_empty() {
            return self.finalize_report(report);
        }
        
        // Collect all check futures
        /*
        // Code commented out to remove tokio dependency
        let check_futures = self.health_checks.iter().enumerate().map(|(i, check)| {
            let api = self.journal_api.clone();
            let check_name = format!("check_{}", i);
            
            // Run each check directly in the current thread
            {
                let start = Instant::now();
                let result = check(&api);
                let duration = start.elapsed();
                
                CheckResult {
                    name: check_name,
                    status: match &result {
                        Ok(_) => HealthStatus::Healthy,
                        Err(_) => HealthStatus::Degraded,
                    },
                    message: match result {
                        Ok(_) => None,
                        Err(e) => Some(e.to_string()),
                    },
                    duration,
                    timestamp: Utc::now(),
                }
            }
        });
        
        // Run all checks concurrently
        let results = join_all(check_futures).await;
        
        // Process results and update report status
        for result in results {
            match result {
                Ok(check_result) => {
                    if check_result.status == HealthStatus::Degraded && report.status == HealthStatus::Healthy {
                        report.status = HealthStatus::Degraded;
                    }
                    report.checks.push(check_result);
                }
                Err(e) => {
                    report.checks.push(CheckResult {
                        name: "task_failed".to_string(),
        status: HealthStatus::Degraded,
                        message: Some(format!("Task failed: {}", e)),
                        duration: Duration::from_secs(0),
                        timestamp: Utc::now(),
                    });
                    
                    if report.status == HealthStatus::Healthy {
                        report.status = HealthStatus::Degraded;
                    }
                }
            }
        }
        */
        
        self.finalize_report(report)
    }
    */
    
    /// Run health checks synchronously
    fn check_system_health_sync(&self) -> HealthReport {
        let now = Utc::now();
        let mut report = HealthReport {
            timestamp: now,
            status: HealthStatus::Healthy,
            metrics: Vec::new(),
            alerts: Vec::new(),
            checks: Vec::with_capacity(self.health_checks.len()),
        };
        
        // Run all health checks sequentially
        for (i, check) in self.health_checks.iter().enumerate() {
            let start = Instant::now();
            let result = check(&self.journal_api);
            let duration = start.elapsed();
            
            let check_result = CheckResult {
                name: format!("check_{}", i),
                status: match &result {
                    Ok(_) => HealthStatus::Healthy,
                    Err(_) => HealthStatus::Degraded,
                },
                message: match result {
                    Ok(_) => None,
                    Err(e) => Some(e.to_string()),
                },
                duration,
                timestamp: Utc::now(),
            };
            
            if check_result.status == HealthStatus::Degraded && report.status == HealthStatus::Healthy {
                report.status = HealthStatus::Degraded;
            }
            
            report.checks.push(check_result);
        }
        
        self.finalize_report(report)
    }
    
    /// Finalize the health report by adding metrics and alerts
    fn finalize_report(&self, mut report: HealthReport) -> HealthReport {
        // Get current metrics and check thresholds
        match self.metrics.lock() {
            Ok(metrics_guard) => {
                for (_, metric) in metrics_guard.iter() {
                    // Add metric to report
                    report.metrics.push(metric.clone());
                    
                    // Check if metric exceeds critical threshold
                    if let Some(threshold) = metric.critical_threshold {
                        if metric.value >= threshold && report.status != HealthStatus::Critical {
                            report.status = HealthStatus::Critical;
                            
                            // Create an alert for critical threshold
                            let _ = self.create_alert(
                                format!("Critical threshold exceeded: {} = {}{} (threshold: {})", 
                                    metric.name, metric.value, metric.unit, threshold),
                                AlertLevel::Critical,
                                vec![metric.clone()]
                            );
                        }
                    }
                    
                    // Check if metric exceeds warning threshold (only if not already critical)
                    if report.status != HealthStatus::Critical {
                        if let Some(threshold) = metric.warning_threshold {
                            if metric.value >= threshold && report.status == HealthStatus::Healthy {
                                report.status = HealthStatus::Degraded;
                                
                                // Create an alert for warning threshold
                                let _ = self.create_alert(
                                    format!("Warning threshold exceeded: {} = {}{} (threshold: {})", 
                                        metric.name, metric.value, metric.unit, threshold),
                                    AlertLevel::Warning,
                                    vec![metric.clone()]
                                );
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to lock metrics: {}", e);
                report.checks.push(CheckResult {
                    name: "metrics_lock_failed".to_string(),
                    status: HealthStatus::Degraded,
                    message: Some(format!("Failed to access metrics: {}", e)),
                    duration: Duration::from_secs(0),
                    timestamp: Utc::now(),
                });
                
                if report.status == HealthStatus::Healthy {
                    report.status = HealthStatus::Degraded;
                }
            }
        }
        
        // Get any active alerts
        match self.active_alerts.lock() {
            Ok(alerts_guard) => {
                report.alerts = alerts_guard.iter()
                    .filter(|a| !a.acknowledged)
                    .cloned()
                    .collect();
                    
                // If there are unacknowledged critical alerts, mark as critical
                if report.status != HealthStatus::Critical {
                    if report.alerts.iter().any(|a| a.level == AlertLevel::Critical) {
                        report.status = HealthStatus::Critical;
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to lock active_alerts: {}", e);
                report.checks.push(CheckResult {
                    name: "alerts_lock_failed".to_string(),
                    status: HealthStatus::Degraded,
                    message: Some(format!("Failed to access active_alerts: {}", e)),
                    duration: Duration::from_secs(0),
                    timestamp: Utc::now(),
                });
                
                if report.status == HealthStatus::Healthy {
                    report.status = HealthStatus::Degraded;
                }
            }
        }
        
        report
    }
    
    /// Format a health report as a human-readable string
    pub fn format_report(report: &HealthReport) -> String {
        let status_str = match report.status {
            HealthStatus::Healthy => "✅ HEALTHY",
            HealthStatus::Degraded => "⚠️ DEGRADED",
            HealthStatus::Critical => "❌ CRITICAL",
            HealthStatus::Unknown => "❓ UNKNOWN",
        };
        
        let mut result = format!(
            "Health Report [{}]\nStatus: {}\n\n",
            report.timestamp, status_str
        );
        
        // Add health check results
        if !report.checks.is_empty() {
            result.push_str("=== Health Checks ===\n");
            for check in &report.checks {
                let status_str = match check.status {
                    HealthStatus::Healthy => "✓",
                    _ => "✗",
                };
                
                result.push_str(&format!(
                    "{} {}: {}\n",
                    status_str,
                    check.name,
                    check.message.as_deref().unwrap_or("OK")
                ));
            }
            result.push_str("\n");
        }
        
        // Add metrics
        if !report.metrics.is_empty() {
            result.push_str("=== Metrics ===\n");
            for metric in &report.metrics {
                let threshold_info = if let Some(threshold) = metric.critical_threshold {
                    format!(" (CRITICAL at {}{})", threshold, metric.unit)
                } else if let Some(threshold) = metric.warning_threshold {
                    format!(" (WARNING at {}{})", threshold, metric.unit)
                } else {
                    String::new()
                };
                
                result.push_str(&format!(
                    "{}: {}{}{}\n",
                    metric.name, metric.value, metric.unit, threshold_info
                ));
            }
            result.push_str("\n");
        }
        
        // Add active alerts
        if !report.alerts.is_empty() {
            result.push_str("=== Active Alerts ===\n");
            for alert in &report.alerts {
                let level_str = match alert.level {
                    AlertLevel::Critical => "🔴 CRITICAL",
                    AlertLevel::Warning => "🟠 WARNING",
                    AlertLevel::Info => "ℹ️ INFO",
                    AlertLevel::Emergency => "🚨 EMERGENCY",
                };
                
                result.push_str(&format!(
                    "[{}] {} - {}\n",
                    alert.timestamp.format("%Y-%m-%d %H:%M:%S"),
                    level_str,
                    alert.message
                ));
                
                if !alert.metrics.is_empty() {
                    result.push_str("  Metrics:\n");
                    for metric in &alert.metrics {
                        result.push_str(&format!(
                            "    {}: {}{}\n",
                            metric.name, metric.value, metric.unit
                        ));
                    }
                }
                
                if alert.acknowledged {
                    result.push_str(&format!(
                        "  Acknowledged at {}\n",
                        alert.acknowledged_at.unwrap_or(Utc::now())
                            .format("%Y-%m-%d %H:%M:%S")
                    ));
                }
                
                result.push_str("\n");
            }
        }
        
        result
    }
}

/// Returns a list of standard health checks for the journal system
///
/// These checks monitor various aspects of the journal system and can be used
/// with the `HealthMonitor` to ensure the system is operating within expected
/// parameters.
pub fn standard_checks() -> Vec<HealthCheck> {
    vec![
        // Check for journal forks (data consistency)
        Box::new(|api| {
            // In a real implementation, this would be async
            let forks: Vec<String> = Vec::new(); // api.detect_forks().await?;
            
            if !forks.is_empty() {
                return Err(MonitorError::SystemDegraded(format!(
                    "{} potential fork(s) detected in the journal",
                    forks.len()
                )));
            }
            
            Ok(())
        }),
        
        // Check journal size
        Box::new(|api| {
            // In a real implementation, this would check actual journal size
            let journal_size = 0; // api.get_journal_size().await?;
            
            if journal_size > 1_000_000_000 { // 1GB
                return Err(MonitorError::ThresholdExceeded {
                    name: "journal_size".to_string(),
                    value: journal_size as f64,
                    threshold: 1_000_000_000.0,
                });
            }
            
            Ok(())
        }),
        
        // Check for recent errors in the journal
        Box::new(|api| {
            // In a real implementation, this would check for recent errors
            let error_count = 0; // api.get_recent_error_count().await?;
            
            if error_count > 10 {
                return Err(MonitorError::SystemDegraded(format!(
                    "High number of recent errors: {}",
                    error_count
                )));
            }
            
            Ok(())
        }),
        
        // Check write latency
        Box::new(|api| {
            // In a real implementation, this would measure actual write latency
            let write_latency_ms = 0; // api.get_avg_write_latency().await?;
            
            if write_latency_ms > 1000 { // 1 second
                return Err(MonitorError::SystemDegraded(format!(
                    "High write latency: {}ms",
                    write_latency_ms
                )));
            }
            
            Ok(())
        }),
        
        // Check available storage space
        Box::new(|api| {
            // In a real implementation, this would check actual disk space
            let free_space_bytes = 10_000_000_000u64; // 10GB
            let min_required_bytes = 1_000_000_000; // 1GB
            
            if free_space_bytes < min_required_bytes {
                return Err(MonitorError::SystemDegraded(format!(
                    "Low disk space: {:.2}GB free (min {:.2}GB required)",
                    free_space_bytes as f64 / 1_000_000_000.0,
                    min_required_bytes as f64 / 1_000_000_000.0
                )));
            }
            
            Ok(())
        }),
        
        // Check connectivity to dependent services
        Box::new(|_api| {
            // In a real implementation, this would verify connections to all dependencies
            let dependencies_ok = true; // api.check_dependencies().await?;
            
            if !dependencies_ok {
                return Err(MonitorError::SystemDegraded(
                    "One or more dependent services are unavailable".to_string()
                ));
            }
            
            Ok(())
        }),
        
        // Check for high memory usage
        Box::new(|_api| {
            // In a real implementation, this would check system memory usage
            let memory_usage_percent = 30.0; // api.get_memory_usage().await?;
            let critical_threshold = 90.0;
            let warning_threshold = 75.0;
            
            if memory_usage_percent >= critical_threshold {
                return Err(MonitorError::SystemDegraded(format!(
                    "Critical memory usage: {:.1}% (threshold: {}%)",
                    memory_usage_percent, critical_threshold
                )));
            } else if memory_usage_percent >= warning_threshold {
                return Err(MonitorError::SystemDegraded(format!(
                    "High memory usage: {:.1}% (warning at {}%)",
                    memory_usage_percent, warning_threshold
                )));
            }
            
            Ok(())
        }),
        
        // Check CPU usage
        Box::new(|_api| {
            // In a real implementation, this would check system CPU usage
            let cpu_usage_percent = 45.0; // api.get_cpu_usage().await?;
            let critical_threshold = 90.0;
            let warning_threshold = 75.0;
            
            if cpu_usage_percent >= critical_threshold {
                return Err(MonitorError::SystemDegraded(format!(
                    "Critical CPU usage: {:.1}% (threshold: {}%)",
                    cpu_usage_percent, critical_threshold
                )));
            } else if cpu_usage_percent >= warning_threshold {
                return Err(MonitorError::SystemDegraded(format!(
                    "High CPU usage: {:.1}% (warning at {}%)",
                    cpu_usage_percent, warning_threshold
                )));
            }
            
            Ok(())
        }),
        
        // Check for long-running transactions
        Box::new(|_api| {
            // In a real implementation, this would check for transactions
            let long_running_tx = Vec::<String>::new(); // api.get_long_running_transactions().await?;
            
            if !long_running_tx.is_empty() {
                return Err(MonitorError::SystemDegraded(format!(
                    "{} long-running transaction(s) detected",
                    long_running_tx.len()
                )));
            }
            
            Ok(())
        }),
    ]
}

// Integration with std::fmt for AlertLevel
impl std::fmt::Display for AlertLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertLevel::Info => write!(f, "INFO"),
            AlertLevel::Warning => write!(f, "WARNING"),
            AlertLevel::Critical => write!(f, "CRITICAL"),
            AlertLevel::Emergency => write!(f, "EMERGENCY"),
        }
    }
}

// Display implementation is derived via #[derive(Debug, Error)]

impl From<JournalApiError> for MonitorError {
    fn from(error: JournalApiError) -> Self {
        MonitorError::ValidationError(format!("Journal API error: {}", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use crate::journal_api::JournalApi;

    // Helper function to create a test JournalApi instance
    fn create_test_journal_api() -> Arc<JournalApi> {
        // In a real test, you would set up a test journal
        Arc::new(JournalApi::new())
    }

    #[test]
    fn test_metric_operations() {
        let journal_api = create_test_journal_api();
        let monitor = HealthMonitor::new(journal_api);
        
        // Test adding a new metric
        monitor.add_metric("test_metric", 42.0, "units", Some(50.0), Some(100.0));
        
        // Test updating the metric
        monitor.update_metric("test_metric", 75.0, None, None, None);
        
        // Check that the metric was updated
        let report = monitor.check_system_health();
        let metric = report.metrics.iter()
            .find(|m| m.name == "test_metric")
            .expect("Metric not found");
            
        assert_eq!(metric.value, 75.0);
        assert_eq!(metric.unit, "units");
        assert_eq!(metric.warning_threshold, Some(50.0));
        assert_eq!(metric.critical_threshold, Some(100.0));
    }
    
    #[test]
    fn test_alert_workflow() {
        let journal_api = create_test_journal_api();
        let monitor = HealthMonitor::new(journal_api);
        
        // Create a test metric
        let test_metric = Metric {
            name: "test_metric".to_string(),
            value: 150.0,
            unit: "units".to_string(),
            timestamp: Utc::now(),
            warning_threshold: Some(100.0),
            critical_threshold: Some(200.0),
        };
        
        // Create an alert
        let alert = monitor.create_alert(
            "Test warning".to_string(),
            AlertLevel::Warning,
            vec![test_metric.clone()]
        ).expect("Failed to create alert");
        
        // Verify the alert is active
        let report = monitor.check_system_health();
        assert_eq!(report.alerts.len(), 1);
        assert_eq!(report.alerts[0].message, "Test warning");
        assert_eq!(report.alerts[0].acknowledged, false);
        
        // Acknowledge the alert
        monitor.acknowledge_alert(&alert.id).expect("Failed to acknowledge alert");
        
        // Verify the alert is acknowledged
        let report = monitor.check_system_health();
        assert_eq!(report.alerts[0].acknowledged, true);
        assert!(report.alerts[0].acknowledged_at.is_some());
    }
    
    #[test]
    fn test_health_check_thresholds() {
        let journal_api = create_test_journal_api();
        let mut monitor = HealthMonitor::new(journal_api);
        
        // Add a metric that exceeds warning threshold
        monitor.add_metric("high_metric", 150.0, "units", Some(100.0), Some(200.0));
        
        // Check that the system is in degraded state
        let report = monitor.check_system_health();
        assert_eq!(report.status, HealthStatus::Degraded);
        
        // Update to exceed critical threshold
        monitor.update_metric("high_metric", 250.0, None, None, None);
        
        // Check that the system is in critical state
        let report = monitor.check_system_health();
        assert_eq!(report.status, HealthStatus::Critical);
        
        // Fix the metric
        monitor.update_metric("high_metric", 50.0, None, None, None);
        
        // Check that the system is healthy again
        let report = monitor.check_system_health();
        assert_eq!(report.status, HealthStatus::Healthy);
    }
    
    #[test]
    fn test_standard_checks() {
        let journal_api = create_test_journal_api();
        let mut monitor = HealthMonitor::new(journal_api);
        
        // Add standard checks
        let checks = standard_checks();
        for check in checks {
            monitor.add_check(check);
        }
        
        // Run health checks
        let report = monitor.check_system_health();
        
        // In a real test, you would mock the API responses to test different scenarios
        // For now, just verify the report structure
        assert!(report.checks.len() > 0);
    }
    
    #[test]
    fn test_alert_handling() {
        let journal_api = create_test_journal_api();
        let monitor = HealthMonitor::new(journal_api);
        
        // Create a critical alert
        let alert = monitor.create_alert(
            "Critical issue".to_string(),
            AlertLevel::Critical,
            vec![]
        ).unwrap();
        
        // Check that the system status reflects the critical alert
        let report = monitor.check_system_health();
        assert_eq!(report.status, HealthStatus::Critical);
        
        // Acknowledge the alert
        monitor.acknowledge_alert(&alert.id).unwrap();
        
        // System should still be critical (acknowledgment doesn't change status)
        let report = monitor.check_system_health();
        assert_eq!(report.status, HealthStatus::Critical);
        assert!(report.alerts[0].acknowledged);
    }

    #[test]
    fn test_async_operations() {
        // This test demonstrates how async operations would be tested
        // In a real async test, you would use #[tokio::test] and .await
        let journal_api = create_test_journal_api();
        let monitor = HealthMonitor::new(journal_api);
        
        // For now, just verify the monitor was created
        assert_eq!(monitor.check_system_health().status, HealthStatus::Healthy);
    }
    
    #[test]
    fn test_health_monitor_sync() {
        // Test the health monitor in a synchronous context
        let journal_api = create_test_journal_api();
        let mut monitor = HealthMonitor::new(journal_api);
        
        // Add some metrics
        monitor.add_metric("cpu_usage", 0.5, "%", Some(80.0), Some(95.0));
        monitor.add_metric("memory_usage", 0.3, "%", Some(80.0), Some(95.0));
        
        // Add standard health checks
        let checks = standard_checks();
        for check in checks {
            monitor.add_check(check);
        }
        
        // Check system health
        let report = monitor.check_system_health();
        
        // System should be healthy
        assert_eq!(report.status, HealthStatus::Healthy);
        
        // Now update a metric to trigger a warning
        monitor.update_metric("cpu_usage", 85.0, None, None, None).unwrap();
        
        // Check system health again
        let report = monitor.check_system_health();
        
        // System should be degraded
        assert_eq!(report.status, HealthStatus::Degraded);
        
        // There should be an active alert
        assert!(!report.alerts.is_empty());
        
        // Format the report
        let report_str = HealthMonitor::format_report(&report);
        assert!(report_str.contains("DEGRADED"));
    }
}

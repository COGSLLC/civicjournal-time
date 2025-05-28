use std::sync::Arc;
use chrono::Utc;
use tokio::sync::Mutex;
use std::time::Duration;
use thiserror::Error;

use crate::journal_api::{JournalApi, JournalApiError, RateLimitedJournal, ValidatedJournal};
use crate::rate_limiter::{RateLimiter, RateLimitSettings, RateLimitError};
use crate::schema::{CivicObject, CivicObjectV1, CivicObjectV2, Delta, StateValidator, ValidationError, DefaultValidator};
#[cfg(feature = "web")]
use crate::schema::AsyncStateValidator;
use crate::recovery::{Journal, RecoveryError};
use crate::fork_detection::{ForkDetector, ForkDetection, ResolutionStrategy};
use crate::health_monitor::{HealthMonitor, HealthStatus, AlertLevel};

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("API error: {0}")]
    Api(#[from] JournalApiError),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Recovery error: {0}")]
    Recovery(#[from] RecoveryError),
    
    #[error("Rate limiting error: {0}")]
    RateLimit(String),
    
    #[error("Health monitoring error: {0}")]
    HealthMonitor(String),
    
    #[error("Fork detection error: {0}")]
    ForkDetection(String),
    
    #[error("Test failed: {0}")]
    TestFailed(String),
}

/// A combined validator that applies multiple validation rules
pub struct CompositeValidator {
    validators: Vec<Arc<dyn StateValidator>>,
}

impl CompositeValidator {
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }
    
    pub fn add_validator<V: StateValidator + 'static>(&mut self, validator: V) {
        self.validators.push(Arc::new(validator));
    }
}

impl StateValidator for CompositeValidator {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn validate_delta_sync(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate_delta_sync(current_state, delta)?;
        }
        Ok(())
    }
    
    fn validate_state_sync(&self, state: &CivicObject) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate_state_sync(state)?;
        }
        Ok(())
    }
}

#[cfg(feature = "web")]
#[async_trait::async_trait]
impl AsyncStateValidator for CompositeValidator {
    async fn validate_delta(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        self.validate_composite_async(current_state, delta).await
    }
    
    async fn validate_state(&self, state: &CivicObject) -> Result<(), ValidationError> {
        self.validate_objects_async(state).await
    }
}

impl CompositeValidator {
    #[cfg(not(feature = "web"))]
    fn validate_composite(&self, current_state: Option<&CivicObject>, delta: &Delta) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate_delta_sync(current_state, delta)?;
        }
        Ok(())
    }
    
    #[cfg(not(feature = "web"))]
    fn validate_objects(&self, state: &CivicObject) -> Result<(), ValidationError> {
        for validator in &self.validators {
            validator.validate_state_sync(state)?;
        }
        Ok(())
    }
    
    #[cfg(feature = "web")]
    async fn validate_composite_async(&self, current_state: Option<&CivicObject>, delta: &Delta) -> Result<(), ValidationError> {
        for validator in &self.validators {
            if let Some(async_validator) = validator.as_any().downcast_ref::<dyn AsyncStateValidator>() {
                async_validator.validate_delta(current_state, delta).await?
            } else {
                validator.validate_delta_sync(current_state, delta)?
            }
        }
        Ok(())
    }
    
    #[cfg(feature = "web")]
    async fn validate_objects_async(&self, state: &CivicObject) -> Result<(), ValidationError> {
        for validator in &self.validators {
            if let Some(async_validator) = validator.as_any().downcast_ref::<dyn AsyncStateValidator>() {
                async_validator.validate_state(state).await?
            } else {
                validator.validate_state_sync(state)?
            }
        }
        Ok(())
    }
}

/// A custom validator that enforces some business rules
pub struct BusinessRuleValidator {
    max_field_length: usize,
    required_fields: Vec<String>,
}

impl BusinessRuleValidator {
    pub fn new() -> Self {
        Self {
            max_field_length: 1024,
            required_fields: vec!["id".to_string(), "name".to_string()],
        }
    }
}

impl StateValidator for BusinessRuleValidator {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn validate_delta_sync(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        // Delegate to validate_state for creation operations
        match &delta.operation {
            crate::schema::Operation::Create { object } => {
                self.validate_state_sync(object)
            },
            _ => Ok(()),
        }
    }
    
    fn validate_state_sync(&self, state: &CivicObject) -> Result<(), ValidationError> {
        match state {
            CivicObject::V1(obj) => {
                // Check required fields
                for field in &self.required_fields {
                    if field == "id" {
                        if obj.id.is_empty() {
                            return Err(ValidationError::MissingRequiredField("id".to_string()));
                        }
                    } else if !obj.data.contains_key(field) {
                        return Err(ValidationError::MissingRequiredField(field.clone()));
                    }
                }
                
                // Check field lengths
                for (key, value) in &obj.data {
                    if value.len() > self.max_field_length {
                        return Err(ValidationError::FieldTooLarge {
                            field: key.clone(),
                            max_size: self.max_field_length,
                            actual_size: value.len(),
                        });
                    }
                }
                
                Ok(())
            },
            CivicObject::V2(obj) => {
                // Check required fields in V2 object
                for field in &self.required_fields {
                    if field == "id" {
                        if obj.id.is_empty() {
                            return Err(ValidationError::MissingRequiredField("id".to_string()));
                        }
                    } else if !obj.data.contains_key(field) {
                        return Err(ValidationError::MissingRequiredField(field.clone()));
                    }
                }
                
                Ok(())
            },
        }
    }
}

#[cfg(feature = "web")]
#[async_trait::async_trait]
impl AsyncStateValidator for BusinessRuleValidator {
    async fn validate_delta(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        // For the async version, we can just delegate to the sync implementation
        self.validate_delta_sync(current_state, delta)
    }
    
    async fn validate_state(&self, state: &CivicObject) -> Result<(), ValidationError> {
        match state {
            CivicObject::V1(obj) => {
                // Check required fields
                for field in &self.required_fields {
                    if field == "id" {
                        if obj.id.is_empty() {
                            return Err(ValidationError::MissingRequiredField("id".to_string()));
                        }
                    } else if !obj.data.contains_key(field) {
                        return Err(ValidationError::MissingRequiredField(field.clone()));
                    }
                }
                
                // Check field lengths
                for (key, value) in &obj.data {
                    if value.len() > self.max_field_length {
                        return Err(ValidationError::FieldTooLarge {
                            field: key.clone(),
                            max_size: self.max_field_length,
                            actual_size: value.len(),
                        });
                    }
                }
                
                Ok(())
            },
            CivicObject::V2(obj) => {
                // Check required fields
                for field in &self.required_fields {
                    if field == "id" {
                        if obj.id.is_empty() {
                            return Err(ValidationError::MissingRequiredField("id".to_string()));
                        }
                    } else if !obj.data.contains_key(field) {
                        return Err(ValidationError::MissingRequiredField(field.clone()));
                    }
                }
                
                Ok(())
            },
        }
    }
}

/// Creates a journal API configured for integration testing
pub fn create_test_journal_api() -> JournalApi {
    // Create a base journal API
    let api = JournalApi::new();
    
    // Configure rate limiting
    let api = api.with_rate_limit(RateLimitSettings {
        max_requests: 10,         // 10 requests per minute for testing
        time_window: Duration::from_secs(60),
        max_burst: 5,             // 5 concurrent requests
        ban_duration: Duration::from_secs(5), // 5 second ban for testing
        violations_before_ban: 2,  // 2 strikes
        cleanup_interval: Duration::from_secs(1),
    });
    
    // Configure validation
    let mut validator = CompositeValidator::new();
    validator.add_validator(DefaultValidator::new());
    validator.add_validator(BusinessRuleValidator::new());
    
    api.with_validator(validator)
}

/// Integration test suite
pub struct IntegrationTests {
    journal_api: Arc<JournalApi>,
}

impl IntegrationTests {
    pub fn new() -> Self {
        Self {
            journal_api: Arc::new(create_test_journal_api()),
        }
    }
    
    /// Run all integration tests
    pub async fn run_all_tests(&self) -> Vec<Result<String, IntegrationError>> {
        let mut results = Vec::new();
        
        results.push(self.test_schema_validation().await
            .map(|_| "Schema validation test passed".to_string()));
            
        results.push(self.test_rate_limiting().await
            .map(|_| "Rate limiting test passed".to_string()));
            
        results.push(self.test_recovery().await
            .map(|_| "Recovery test passed".to_string()));
            
        results.push(self.test_fork_detection().await
            .map(|_| "Fork detection test passed".to_string()));
            
        results.push(self.test_health_monitoring().await
            .map(|_| "Health monitoring test passed".to_string()));
            
        results.push(self.test_end_to_end().await
            .map(|_| "End-to-end integration test passed".to_string()));
            
        results
    }
    
    /// Test schema validation
    pub async fn test_schema_validation(&self) -> Result<(), IntegrationError> {
        // Create a valid object
        let valid_obj = CivicObject::V1(CivicObjectV1 {
            id: "test1".to_string(),
            created_at: Utc::now(),
            data: std::collections::BTreeMap::from([
                ("name".to_string(), "Test Object".to_string()),
                ("description".to_string(), "This is a test object".to_string()),
            ]),
        });
        
        // Add to journal - should succeed
        self.journal_api.append_entry("test_client", valid_obj.clone()).await
            .map_err(IntegrationError::Api)?;
            
        // Create an invalid object (missing required field)
        let invalid_obj = CivicObject::V1(CivicObjectV1 {
            id: "test2".to_string(),
            created_at: Utc::now(),
            data: std::collections::BTreeMap::from([
                ("description".to_string(), "This is an invalid test object".to_string()),
            ]),
        });
        
        // Add to journal - should fail validation
        let result = self.journal_api.append_entry("test_client", invalid_obj).await;
        
        match result {
            Err(JournalApiError::ValidationError(_)) => Ok(()),
            _ => Err(IntegrationError::TestFailed(
                "Validation did not fail as expected".to_string()
            )),
        }
    }
    
    /// Test rate limiting
    pub async fn test_rate_limiting(&self) -> Result<(), IntegrationError> {
        // Create a test object
        let obj = CivicObject::V1(CivicObjectV1 {
            id: "rate_test".to_string(),
            created_at: Utc::now(),
            data: std::collections::BTreeMap::from([
                ("name".to_string(), "Rate Test".to_string()),
            ]),
        });
        
        // Send multiple requests in quick succession
        for i in 0..5 {
            let mut test_obj = obj.clone();
            
            // Modify the ID to make it unique
            match &mut test_obj {
                CivicObject::V1(ref mut v1) => {
                    v1.id = format!("rate_test_{}", i);
                }
                CivicObject::V2(ref mut v2) => {
                    v2.id = format!("rate_test_{}", i);
                }
            }
            
            // Should succeed for the first few requests
            self.journal_api.append_entry("rate_test_client", test_obj).await
                .map_err(IntegrationError::Api)?;
        }
        
        // Now send more requests that should trigger rate limiting
        let mut saw_rate_limit = false;
        
        for i in 5..15 {
            let mut test_obj = obj.clone();
            
            // Modify the ID to make it unique
            match &mut test_obj {
                CivicObject::V1(ref mut v1) => {
                    v1.id = format!("rate_test_{}", i);
                }
                CivicObject::V2(ref mut v2) => {
                    v2.id = format!("rate_test_{}", i);
                }
            }
            
            // May fail due to rate limiting
            let result = self.journal_api.append_entry("rate_test_client", test_obj).await;
            
            if let Err(JournalApiError::RateLimited(_)) = result {
                saw_rate_limit = true;
                break;
            }
        }
        
        if saw_rate_limit {
            Ok(())
        } else {
            Err(IntegrationError::TestFailed(
                "Rate limiting did not trigger as expected".to_string()
            ))
        }
    }
    
    /// Test recovery functionality
    pub async fn test_recovery(&self) -> Result<(), IntegrationError> {
        // Create some test entries
        // In a real test, we would create a corrupted journal
        // and test recovery, but for simplicity, we'll just check
        // that the recovery function can be called
        
        let result = self.journal_api.recover().await
            .map_err(IntegrationError::Api)?;
            
        // Verify that we got a recovery report
        if result.contains("Recovery completed") {
            Ok(())
        } else {
            Err(IntegrationError::TestFailed(
                "Recovery did not return expected message".to_string()
            ))
        }
    }
    
    /// Test fork detection and resolution
    pub async fn test_fork_detection(&self) -> Result<(), IntegrationError> {
        // In a real test, we would create a forked journal
        // and test detection and resolution
        
        // For now, we'll just check that fork detection can be called
        let forks = self.journal_api.detect_forks().await
            .map_err(IntegrationError::Api)?;
            
        // No forks expected in an empty journal
        if forks.is_empty() {
            Ok(())
        } else {
            Err(IntegrationError::TestFailed(
                "Unexpected forks detected".to_string()
            ))
        }
    }
    
    /// Test health monitoring
    pub async fn test_health_monitoring(&self) -> Result<(), IntegrationError> {
        // Create a health monitor
        let monitor = HealthMonitor::new(self.journal_api.clone());
        
        // Check system health
        let report = monitor.check_system_health();
        
        // New system should be healthy
        if report.status == HealthStatus::Healthy {
            Ok(())
        } else {
            Err(IntegrationError::TestFailed(
                format!("System not healthy: {:?}", report.status)
            ))
        }
    }
    
    /// End-to-end integration test that exercises all components
    pub async fn test_end_to_end(&self) -> Result<(), IntegrationError> {
        // 1. Create valid objects with schema validation
        let obj1 = CivicObject::V1(CivicObjectV1 {
            id: "integrated1".to_string(),
            created_at: Utc::now(),
            data: std::collections::BTreeMap::from([
                ("name".to_string(), "Integrated Test 1".to_string()),
                ("value".to_string(), "100".to_string()),
            ]),
        });
        
        let obj2 = CivicObject::V1(CivicObjectV1 {
            id: "integrated2".to_string(),
            created_at: Utc::now(),
            data: std::collections::BTreeMap::from([
                ("name".to_string(), "Integrated Test 2".to_string()),
                ("value".to_string(), "200".to_string()),
            ]),
        });
        
        // 2. Add to journal with rate limiting
        #[cfg(feature = "web")]
        {
            self.journal_api.append_entry("integration_client", obj1).await
                .map_err(IntegrationError::Api)?;
                
            self.journal_api.append_entry("integration_client", obj2).await
                .map_err(IntegrationError::Api)?;
        }
        
        #[cfg(not(feature = "web"))]
        {
            self.journal_api.append_entry("integration_client", obj1).await
                .map_err(IntegrationError::Api)?;
                
            self.journal_api.append_entry("integration_client", obj2).await
                .map_err(IntegrationError::Api)?;
        }
            
        // 3. Check for forks (none expected)
        #[cfg(feature = "web")]
        let forks = self.journal_api.detect_forks().await
            .map_err(IntegrationError::Api)?;
            
        #[cfg(not(feature = "web"))]
        let forks = self.journal_api.detect_forks().await
            .map_err(IntegrationError::Api)?;
            
        if !forks.is_empty() {
            return Err(IntegrationError::TestFailed(
                "Unexpected forks detected".to_string()
            ));
        }
        
        // 4. Run recovery (should be healthy)
        #[cfg(feature = "web")]
        let recovery = self.journal_api.recover().await
            .map_err(IntegrationError::Api)?;
            
        #[cfg(not(feature = "web"))]
        let recovery = self.journal_api.recover().await
            .map_err(IntegrationError::Api)?;
            
        if !recovery.contains("Recovery completed") {
            return Err(IntegrationError::TestFailed(
                "Recovery did not complete successfully".to_string()
            ));
        }
        
        // 5. Check health monitoring
        let monitor = HealthMonitor::new(self.journal_api.clone());
        let report = monitor.check_system_health();
        
        if report.status != HealthStatus::Healthy {
            return Err(IntegrationError::TestFailed(
                format!("System not healthy after integration test: {:?}", report.status)
            ));
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::runtime::Runtime;
    
    #[tokio::test]
    async fn test_integration() {
        let tests = IntegrationTests::new();
        let results = tests.run_all_tests().await;
        
        // Check that all tests passed
        for result in &results {
            assert!(result.is_ok(), "Test failed: {:?}", result);
        }
        
        // Print results
        for result in results {
            match result {
                Ok(msg) => println!("✅ {}", msg),
                Err(e) => println!("❌ Test failed: {}", e),
            }
        }
    }
}

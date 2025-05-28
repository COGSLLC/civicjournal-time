use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "schema_version")]
pub enum CivicObject {
    #[serde(rename = "1.0")]
    V1(CivicObjectV1),
    #[serde(rename = "2.0")]
    V2(CivicObjectV2),
}

impl CivicObject {
    pub fn schema_version(&self) -> &'static str {
        match self {
            CivicObject::V1(_) => "1.0",
            CivicObject::V2(_) => "2.0",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            CivicObject::V1(obj) => &obj.id,
            CivicObject::V2(obj) => &obj.id,
        }
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        match self {
            CivicObject::V1(obj) => obj.created_at,
            CivicObject::V2(obj) => obj.created_at,
        }
    }

    pub fn hash(&self) -> [u8; 32] {
        // This is a placeholder for a proper hash calculation
        let mut result = [0u8; 32];
        // In a real implementation, we would serialize and hash the object
        result
    }
}

// Version 1.0 of the object schema
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CivicObjectV1 {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub data: BTreeMap<String, String>, // Simple key-value store
}

// Version 2.0 with additional validation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CivicObjectV2 {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub data: BTreeMap<String, FieldValue>, // Typed values
    pub metadata: BTreeMap<String, FieldValue>,
    pub required_fields: Vec<String>,
}

impl FieldValue {
    /// Returns the length/size of the field value in bytes
    pub fn len(&self) -> usize {
        match self {
            FieldValue::String(s) => s.len(),
            FieldValue::Number(_) => 8, // f64 is 8 bytes
            FieldValue::Boolean(_) => 1, // bool is 1 byte
            FieldValue::DateTime(_) => 8, // DateTime<Utc> is 8 bytes
            FieldValue::Reference(s) => s.len(),
        }
    }
}

// Supported field types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value")]
pub enum FieldValue {
    String(String),
    Number(f64),
    Boolean(bool),
    DateTime(DateTime<Utc>),
    Reference(String), // Reference to another object
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Missing required field: {0}")]
    MissingRequiredField(String),
    
    #[error("Field too large: {field} (max: {max_size}, actual: {actual_size})")]
    FieldTooLarge {
        field: String,
        max_size: usize,
        actual_size: usize,
    },
    
    #[error("Invalid field: {0}")]
    InvalidField(String),
    
    #[error("Type mismatch: expected {expected} for field {field}, got {actual}")]
    TypeMismatch {
        field: String,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Error)]
pub enum MigrationError {
    #[error("No migration path from {from} to {to}")]
    NoMigrationPath { from: String, to: String },
    
    #[error("Invalid source version: expected {expected}, got {actual}")]
    InvalidSourceVersion { expected: String, actual: String },
    
    #[error("Migration failed: {0}")]
    MigrationFailed(String),
}

// Delta represents a change to an object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delta {
    pub operation: Operation,
    pub timestamp: DateTime<Utc>,
    pub previous_hash: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    Create {
        object: CivicObject,
    },
    Update {
        field: String,
        value: FieldValue,
    },
    Delete {
        field: String,
    },
}

impl Delta {
    pub fn new(operation: Operation, timestamp: DateTime<Utc>, previous_hash: [u8; 32]) -> Self {
        Self {
            operation,
            timestamp,
            previous_hash,
        }
    }
}

// Base synchronous validator trait
pub trait StateValidator: Send + Sync {
    fn validate_delta_sync(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError>;
    
    fn validate_state_sync(&self, state: &CivicObject) -> Result<(), ValidationError>;
    
    fn as_any(&self) -> &dyn std::any::Any;
}

// Async validator trait - only available when the web feature is enabled
#[cfg(feature = "web")]
#[async_trait::async_trait]
pub trait AsyncStateValidator: StateValidator {
    async fn validate_delta(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError>;
    
    async fn validate_state(&self, state: &CivicObject) -> Result<(), ValidationError>;
}

pub struct DefaultValidator {
    pub max_field_size: usize,
    pub allowed_field_types: HashMap<String, Vec<String>>,
}

impl DefaultValidator {
    pub fn new() -> Self {
        Self {
            max_field_size: 1024 * 10, // 10KB max field size
            allowed_field_types: HashMap::new(),
        }
    }
}

impl StateValidator for DefaultValidator {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn validate_delta_sync(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        // Implementation of sync version
        match &delta.operation {
            Operation::Create { object } => {
                self.validate_state_sync(object)
            },
            Operation::Update { field, value } => {
                // For updates, we need a current state
                let current = current_state.ok_or(ValidationError::InvalidField(
                    "Cannot update non-existent object".to_string()
                ))?;
                
                // Validate field name
                if field.is_empty() {
                    return Err(ValidationError::InvalidField("Field name cannot be empty".to_string()));
                }
                
                // Check field size
                if let FieldValue::String(s) = value {
                    if s.len() > self.max_field_size {
                        return Err(ValidationError::FieldTooLarge {
                            field: field.clone(),
                            max_size: self.max_field_size,
                            actual_size: s.len(),
                        });
                    }
                }
                
                // Validate type against allowed types if configured
                if let Some(allowed_types) = self.allowed_field_types.get(field) {
                    let type_name = match value {
                        FieldValue::String(_) => "String",
                        FieldValue::Number(_) => "Number",
                        FieldValue::Boolean(_) => "Boolean",
                        FieldValue::DateTime(_) => "DateTime",
                        FieldValue::Reference(_) => "Reference",
                    };
                    
                    if !allowed_types.iter().any(|t| t == type_name) {
                        return Err(ValidationError::TypeMismatch {
                            field: field.clone(),
                            expected: allowed_types.join(" or "),
                            actual: type_name.to_string(),
                        });
                    }
                }
                
                Ok(())
            },
            Operation::Delete { field } => {
                // For deletes, we need a current state
                let current = current_state.ok_or(ValidationError::InvalidField(
                    "Cannot delete field from non-existent object".to_string()
                ))?;
                
                // Validate field name
                if field.is_empty() {
                    return Err(ValidationError::InvalidField("Field name cannot be empty".to_string()));
                }
                
                // Check if the field is required
                match current {
                    CivicObject::V2(obj) => {
                        if obj.required_fields.contains(field) {
                            return Err(ValidationError::MissingRequiredField(field.clone()));
                        }
                    },
                    _ => {}, // V1 doesn't have required fields
                }
                
                Ok(())
            },
        }
    }
    
    fn validate_state_sync(&self, state: &CivicObject) -> Result<(), ValidationError> {
        match state {
            CivicObject::V1(obj) => self.validate_v1_sync(obj),
            CivicObject::V2(obj) => self.validate_v2_sync(obj),
        }
    }
}

// Implement the async trait for web builds
#[cfg(feature = "web")]
#[async_trait::async_trait]
impl AsyncStateValidator for DefaultValidator {
    async fn validate_delta(
        &self,
        current_state: Option<&CivicObject>,
        delta: &Delta,
    ) -> Result<(), ValidationError> {
        match &delta.operation {
            Operation::Create { object } => {
                // Validate new object
                self.validate_state(object).await
            }
            Operation::Update { field, value } => {
                if let Some(state) = current_state {
                    // Check if field exists in schema version
                    match state {
                        CivicObject::V1(obj) => {
                            if !obj.data.contains_key(field) {
                                return Err(ValidationError::InvalidField(format!(
                                    "Field {} does not exist in object schema",
                                    field
                                )));
                            }
                            
                            // Check field size for string values
                            if let FieldValue::String(s) = value {
                                if s.len() > self.max_field_size {
                                    return Err(ValidationError::FieldTooLarge {
                                        field: field.clone(),
                                        max_size: self.max_field_size,
                                        actual_size: s.len(),
                                    });
                                }
                            }
                        }
                        CivicObject::V2(obj) => {
                            if !obj.data.contains_key(field) {
                                return Err(ValidationError::InvalidField(format!(
                                    "Field {} does not exist in object schema",
                                    field
                                )));
                            }
                            
                            // Check field type if specified in allowed_field_types
                            if let Some(allowed_types) = self.allowed_field_types.get(field) {
                                let value_type = match value {
                                    FieldValue::String(_) => "string",
                                    FieldValue::Number(_) => "number",
                                    FieldValue::Boolean(_) => "boolean",
                                    FieldValue::DateTime(_) => "datetime",
                                    FieldValue::Reference(_) => "reference",
                                };
                                
                                if !allowed_types.iter().any(|t| t == value_type) {
                                    return Err(ValidationError::TypeMismatch {
                                        field: field.clone(),
                                        expected: allowed_types.join(" or "),
                                        actual: value_type.to_string(),
                                    });
                                }
                            }
                            
                            // Check field size for string values
                            if let FieldValue::String(s) = value {
                                if s.len() > self.max_field_size {
                                    return Err(ValidationError::FieldTooLarge {
                                        field: field.clone(),
                                        max_size: self.max_field_size,
                                        actual_size: s.len(),
                                    });
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            Operation::Delete { field } => {
                if let Some(state) = current_state {
                    // Check if field is required
                    match state {
                        CivicObject::V1(_) => Ok(()),
                        CivicObject::V2(obj) => {
                            if obj.required_fields.contains(field) {
                                return Err(ValidationError::InvalidField(format!(
                                    "Cannot delete required field {}",
                                    field
                                )));
                            }
                            Ok(())
                        }
                    }
                } else {
                    Err(ValidationError::InvalidField(
                        "Cannot delete field from non-existent object".to_string(),
                    ))
                }
            }
        }
    }

    async fn validate_state(&self, state: &CivicObject) -> Result<(), ValidationError> {
        // For async implementation, we can just call the sync version
        // since our validation logic is not actually async-dependent
        self.validate_state_sync(state)
    }
}

impl DefaultValidator {
    // Sync validation methods
    fn validate_v1_sync(&self, obj: &CivicObjectV1) -> Result<(), ValidationError> {
        // Basic validation for V1 objects
        if obj.id.is_empty() {
            return Err(ValidationError::InvalidField("id cannot be empty".into()));
        }
        
        // Validate field sizes
        for (key, value) in &obj.data {
            if value.len() > self.max_field_size {
                return Err(ValidationError::FieldTooLarge {
                    field: key.clone(),
                    max_size: self.max_field_size,
                    actual_size: value.len(),
                });
            }
        }
        
        Ok(())
    }

    fn validate_v2_sync(&self, obj: &CivicObjectV2) -> Result<(), ValidationError> {
        // Basic validation for V2 objects
        if obj.id.is_empty() {
            return Err(ValidationError::InvalidField("id cannot be empty".into()));
        }
        
        // V2 specific validations
        if obj.metadata.is_empty() {
            return Err(ValidationError::InvalidField("metadata required for V2".into()));
        }
        
        // Validate field sizes for data
        for (key, value) in &obj.data {
            let value_size = value.len();
            if value_size > self.max_field_size {
                return Err(ValidationError::FieldTooLarge {
                    field: key.clone(),
                    max_size: self.max_field_size,
                    actual_size: value_size,
                });
            }
        }
        
        // Validate field sizes for metadata
        for (key, value) in &obj.metadata {
            let value_size = value.len();
            if value_size > self.max_field_size {
                return Err(ValidationError::FieldTooLarge {
                    field: format!("metadata.{}", key),
                    max_size: self.max_field_size,
                    actual_size: value_size,
                });
            }
        }
        
        // Validate required fields are present
        for field in &obj.required_fields {
            if !obj.data.contains_key(field) {
                return Err(ValidationError::MissingRequiredField(field.clone()));
            }
        }
        
        Ok(())
    }
    
    // Async validation methods (only available with web feature)
    #[cfg(feature = "web")]
    async fn validate_v1(&self, obj: &CivicObjectV1) -> Result<(), ValidationError> {
        // For async implementation, delegate to the sync version
        self.validate_v1_sync(obj)
    }
    
    #[cfg(feature = "web")]
    async fn validate_v2(&self, obj: &CivicObjectV2) -> Result<(), ValidationError> {
        // For async implementation, delegate to the sync version
        self.validate_v2_sync(obj)
    }

    // Additional validation for V2 objects that can be called from validate_v2_sync
    fn validate_v2_extended(&self, obj: &CivicObjectV2) -> Result<(), ValidationError> {
        // Validate required fields
        for field in &obj.required_fields {
            if !obj.data.contains_key(field) {
                return Err(ValidationError::MissingRequiredField(field.clone()));
            }
        }

        // Validate field types and constraints
        for (key, value) in &obj.data {
            match value {
                FieldValue::String(s) if s.len() > self.max_field_size => {
                    return Err(ValidationError::FieldTooLarge {
                        field: key.clone(),
                        max_size: self.max_field_size,
                        actual_size: s.len(),
                    });
                }
                FieldValue::Number(n) if !n.is_finite() => {
                    return Err(ValidationError::InvalidField(
                        format!("Field {} contains non-finite number", key)
                    ));
                }
                _ => {} // Valid
            }
        }

        Ok(())
    }
}

pub struct SchemaMigrator {
    migrations: HashMap<String, Box<dyn Migration>>,
}

impl SchemaMigrator {
    pub fn new() -> Self {
        let mut migrations = HashMap::new();
        
        // Register migrations
        migrations.insert(
            "1.0_to_2.0".to_string(),
            Box::new(V1ToV2Migration) as Box<dyn Migration>
        );
        
        Self { migrations }
    }

    pub async fn migrate(
        &self,
        object: CivicObject,
        target_version: &str,
    ) -> Result<CivicObject, MigrationError> {
        let mut current = object;
        
        while current.schema_version() != target_version {
            let migration_key = format!("{}_to_{}", current.schema_version(), target_version);
            
            if let Some(migration) = self.migrations.get(&migration_key) {
                current = migration.apply(current).await?;
            } else {
                return Err(MigrationError::NoMigrationPath {
                    from: current.schema_version().to_string(),
                    to: target_version.to_string(),
                });
            }
        }
        
        Ok(current)
    }
}

#[async_trait]
pub trait Migration: Send + Sync {
    fn from_version(&self) -> &'static str;
    fn to_version(&self) -> &'static str;
    async fn apply(&self, object: CivicObject) -> Result<CivicObject, MigrationError>;
}

struct V1ToV2Migration;

#[async_trait]
impl Migration for V1ToV2Migration {
    fn from_version(&self) -> &'static str { "1.0" }
    fn to_version(&self) -> &'static str { "2.0" }

    async fn apply(&self, object: CivicObject) -> Result<CivicObject, MigrationError> {
        if let CivicObject::V1(v1) = object {
            // Convert V1 to V2
            let now = Utc::now();
            let mut data = BTreeMap::new();
            
            // Convert simple strings to typed values
            for (k, v) in v1.data {
                // Try to infer type
                let value = if let Ok(n) = v.parse::<f64>() {
                    FieldValue::Number(n)
                } else if let Ok(b) = v.parse::<bool>() {
                    FieldValue::Boolean(b)
                } else if let Ok(dt) = DateTime::parse_from_rfc3339(&v) {
                    FieldValue::DateTime(dt.into())
                } else {
                    FieldValue::String(v)
                };
                
                data.insert(k, value);
            }
            
            Ok(CivicObject::V2(CivicObjectV2 {
                id: v1.id,
                created_at: v1.created_at,
                updated_at: now,
                data,
                metadata: BTreeMap::new(), // Add empty metadata
                required_fields: vec!["id".to_string()], // At minimum require ID
            }))
        } else {
            Err(MigrationError::InvalidSourceVersion {
                expected: "1.0".to_string(),
                actual: object.schema_version().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_validation() {
        let validator = DefaultValidator::new();
        
        // Valid V1 object
        let v1 = CivicObject::V1(CivicObjectV1 {
            id: "test".to_string(),
            created_at: Utc::now(),
            data: BTreeMap::from([("name".to_string(), "Test".to_string())]),
        });
        
        assert!(validator.validate_state(&v1).await.is_ok());
        
        // Invalid V1 object (empty ID)
        let invalid_v1 = CivicObject::V1(CivicObjectV1 {
            id: "".to_string(),
            created_at: Utc::now(),
            data: BTreeMap::new(),
        });
        
        assert!(validator.validate_state(&invalid_v1).await.is_err());
        
        // Valid V2 object
        let v2 = CivicObject::V2(CivicObjectV2 {
            id: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            data: BTreeMap::from([
                ("name".to_string(), FieldValue::String("Test".to_string())),
                ("count".to_string(), FieldValue::Number(42.0)),
            ]),
            required_fields: vec!["id".to_string(), "name".to_string()],
        });
        
        assert!(validator.validate_state(&v2).await.is_ok());
        
        // Invalid V2 object (missing required field)
        let invalid_v2 = CivicObject::V2(CivicObjectV2 {
            id: "test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            data: BTreeMap::from([
                ("count".to_string(), FieldValue::Number(42.0)),
            ]),
            required_fields: vec!["id".to_string(), "name".to_string()],
        });
        
        assert!(validator.validate_state(&invalid_v2).await.is_err());
    }
    
    #[tokio::test]
    async fn test_migration() {
        let migrator = SchemaMigrator::new();
        
        // Create a V1 object
        let v1 = CivicObject::V1(CivicObjectV1 {
            id: "test".to_string(),
            created_at: Utc::now(),
            data: BTreeMap::from([
                ("name".to_string(), "Test".to_string()),
                ("count".to_string(), "42".to_string()),
                ("active".to_string(), "true".to_string()),
                ("date".to_string(), "2023-01-01T00:00:00Z".to_string()),
            ]),
        });
        
        // Migrate to V2
        let v2 = migrator.migrate(v1, "2.0").await.unwrap();
        
        // Verify migration results
        match v2 {
            CivicObject::V2(obj) => {
                assert_eq!(obj.id, "test");
                assert_eq!(obj.required_fields, vec!["id"]);
                
                // Check inferred types
                match &obj.data["name"] {
                    FieldValue::String(s) => assert_eq!(s, "Test"),
                    _ => panic!("Expected string type for name"),
                }
                
                match &obj.data["count"] {
                    FieldValue::Number(n) => assert_eq!(*n, 42.0),
                    _ => panic!("Expected number type for count"),
                }
                
                match &obj.data["active"] {
                    FieldValue::Boolean(b) => assert!(b),
                    _ => panic!("Expected boolean type for active"),
                }
                
                match &obj.data["date"] {
                    FieldValue::DateTime(_) => {} // Just check it's a datetime
                    _ => panic!("Expected datetime type for date"),
                }
            }
            _ => panic!("Expected V2 object"),
        }
    }
}

use civicjournal_time::types::{
    CompressionAlgorithm,
    StorageType,
    LogLevel,
    ConfigError,
};
use civicjournal_time::error::CJError;
use std::str::FromStr;

#[test]
fn test_compression_display() {
    assert_eq!(CompressionAlgorithm::default().to_string(), "zstd");
    assert_eq!(CompressionAlgorithm::Lz4.to_string(), "lz4");
    assert_eq!(CompressionAlgorithm::Snappy.to_string(), "snappy");
    assert_eq!(CompressionAlgorithm::None.to_string(), "none");
}

#[test]
fn test_storage_type_parse_and_display() {
    assert_eq!(StorageType::from_str("memory").unwrap(), StorageType::Memory);
    assert_eq!(StorageType::from_str("file").unwrap(), StorageType::File);
    assert!(StorageType::from_str("other").is_err());
    assert_eq!(StorageType::default().to_string(), "file");
}

#[test]
fn test_log_level_display_and_parse() {
    assert_eq!(LogLevel::Debug.to_string(), "debug");
    assert_eq!("error".parse::<LogLevel>().unwrap(), LogLevel::Error);
    assert!("bogus".parse::<LogLevel>().is_err());
}

#[test]
fn test_config_error_invalid_value_helper() {
    let err = ConfigError::invalid_value("f", 42, "nope");
    match err {
        ConfigError::InvalidValue { field, value, reason } => {
            assert_eq!(field, "f");
            assert_eq!(value, "42");
            assert_eq!(reason, "nope");
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn test_cjerror_helper_constructors() {
    let e = CJError::invalid_input("bad");
    assert!(matches!(e, CJError::InvalidInput(ref s) if s == "bad"));

    let e = CJError::not_found("x");
    assert!(matches!(e, CJError::NotFound(ref s) if s == "x"));

    let e = CJError::not_supported("y");
    assert!(matches!(e, CJError::NotSupported(ref s) if s == "y"));

    let e = CJError::timeout("z");
    assert!(matches!(e, CJError::Timeout(ref s) if s == "z"));

    let e = CJError::already_in_use("r");
    assert!(matches!(e, CJError::AlreadyInUse(ref s) if s == "r"));

    let e = CJError::not_initialized("n");
    assert!(matches!(e, CJError::NotInitialized(ref s) if s == "n"));
}

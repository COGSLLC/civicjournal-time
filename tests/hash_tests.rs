use civicjournal_time::core::hash::{sha256_hash, sha256_hash_concat};
use hex;

#[test]
fn test_sha256_hash_empty() {
    let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let actual = sha256_hash(b"");
    assert_eq!(hex::encode(actual), expected);
}

#[test]
fn test_sha256_hash_concat_multiple() {
    let slices: &[&[u8]] = &[b"foo", b"bar", b"baz"];
    let expected = "97df3588b5a3f24babc3851b372f0ba71a9dcdded43b14b9d06961bfc1707d9d";
    let actual = sha256_hash_concat(slices);
    assert_eq!(hex::encode(actual), expected);
}

#[test]
fn test_sha256_hash_concat_empty() {
    let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    let actual = sha256_hash_concat(&[]);
    assert_eq!(hex::encode(actual), expected);
}

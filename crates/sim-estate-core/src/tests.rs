use super::*;
use std::collections::BTreeMap;
#[test]
fn symbols_reject_injection_alphabet() {
    for bad in ["", "a b", "a;b", "a\0b", "påth", "../x", "x=y", "*", "x:y"] {
        assert!(Symbol::new(bad).is_err(), "accepted {bad:?}");
    }
}
#[test]
fn all_portable_records_round_trip_and_contain_no_private_keys() {
    let target = Target {
        version: 1,
        id: Symbol::new("node/a").unwrap(),
        labels: BTreeMap::new(),
    };
    let inventory = SanitizedInventory {
        version: 1,
        revision: 7,
        targets: vec![target],
    };
    let encoded = serde_json::to_vec(&inventory).unwrap();
    assert_eq!(
        serde_json::from_slice::<SanitizedInventory>(&encoded).unwrap(),
        inventory
    );
    let text = String::from_utf8(encoded).unwrap();
    for forbidden in [
        "password",
        "secret",
        "token",
        "stdout",
        "path",
        "environment",
    ] {
        assert!(!text.contains(forbidden));
    }
}
#[test]
fn unknown_fields_fail_closed() {
    assert!(
        serde_json::from_str::<Target>(r#"{"version":1,"id":"node/a","labels":{},"path":"/tmp"}"#)
            .is_err()
    );
}

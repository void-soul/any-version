use super::*;
use serde_json::json;
fn pem() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/cert-deployment/tests/fixtures/identity-cert.pem"
    ))
}
#[test]
fn fuzzy_search_does_not_pick_first_and_exact_site_must_be_unique() {
    let items = vec![
        json!({"id":1,"name":"not-target.invalid"}),
        json!({"id":2,"name":"target.invalid"}),
    ];
    assert_eq!(unique_site(&items, "TARGET.INVALID.").unwrap(), "2");
    assert!(unique_site(&items[..1], "target.invalid").is_err());
    let duplicate = vec![
        json!({"id":1,"primaryDomain":"target.invalid"}),
        json!({"id":2,"primaryDomain":"target.invalid"}),
    ];
    assert!(unique_site(&duplicate, "target.invalid").is_err());
}
#[test]
fn primary_domain_cannot_be_overridden_by_an_unrelated_friendly_name() {
    assert!(unique_site(
        &[json!({"id":1,"name":"target.invalid","primaryDomain":"different.invalid"})],
        "target.invalid"
    )
    .is_err());
}
#[test]
fn selected_ssl_requires_both_determined_id_and_exact_leaf_fingerprint() {
    let expected = fingerprint(pem()).unwrap();
    assert!(verify_ssl(&json!({"id":2,"pem":pem()}), "2", &expected).is_ok());
    assert!(verify_ssl(&json!({"id":3,"pem":pem()}), "2", &expected).is_err());
    assert!(verify_ssl(&json!({"id":2,"pem":pem()}), "2", "different fingerprint").is_err());
    assert!(verify_ssl(
        &json!({"id":2,"description":"same-description"}),
        "2",
        &expected
    )
    .is_err());
}
#[test]
fn record_ids_are_positive_integers_not_arbitrary_url_segments() {
    for id in [json!(-1), json!(0), json!("../other"), json!(null)] {
        assert!(record_id(&json!({"id":id})).is_err());
    }
    assert_eq!(record_id(&json!({"id":"42"})).unwrap(), "42");
}
#[test]
fn fingerprint_is_stable_for_equivalent_pem_line_endings() {
    assert_eq!(
        fingerprint(pem()).unwrap(),
        fingerprint(&pem().replace('\n', "\r\n")).unwrap()
    );
}

#[test]
fn matching_description_alone_cannot_select_an_unverified_or_ambiguous_ssl() {
    let expected = fingerprint(pem()).unwrap();
    let correct = json!({"id":7,"pem":pem(),"description":"deployment"});
    let wrong = json!({"id":1,"pem":"-----BEGIN CERTIFICATE-----\nYWJj\n-----END CERTIFICATE-----","description":"deployment"});
    assert_eq!(
        unique_fingerprinted_ssl(&[wrong, correct.clone()], &expected).unwrap(),
        "7"
    );
    assert!(
        unique_fingerprinted_ssl(&[correct.clone(), json!({"id":8,"pem":pem()})], &expected)
            .is_err()
    );
    assert!(unique_fingerprinted_ssl(&[], &expected).is_err());
    assert!(
        unique_fingerprinted_ssl(&[json!({"id":7,"description":"deployment"})], &expected).is_err()
    );
}

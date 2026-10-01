//! Tests for generated typed call views.

#![forbid(unsafe_code)]

use wasm_junction::{Call, Caller, TypedCall};

wasm_junction::bindgen!({ path: "tests/fixtures/host/wit" });

#[test]
fn typed_views_round_trip_arguments_and_results() {
    let call = Call::new(
        Caller::Host,
        "journal",
        notes::INTERFACE,
        "search",
        notes::Search {
            query: "rust".into(),
            limit: 2,
        }
        .into_vals(),
    );
    let search = call.view::<notes::Search>().unwrap().unwrap();
    assert_eq!((search.query.as_str(), search.limit), ("rust", 2));

    let result = Err(notes::AccessError::Denied);
    assert_eq!(
        notes::Read::decode_output(&notes::Read::output(result.clone())).unwrap(),
        result
    );
    assert!(notes::Search::from_vals(&[]).is_err());
    notes::Clear::decode_output(&notes::Clear::output(())).unwrap();
}

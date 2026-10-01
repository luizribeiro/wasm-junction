//! Tests for generated typed call views.

#![forbid(unsafe_code)]

mod support;

use std::sync::Arc;

use wasm_junction::{Call, CallContext, Caller, TypedCall};

wasm_junction::bindgen!({ path: "tests/fixtures/host/wit" });

struct Journal;

impl notes::Host for Journal {
    fn read(&self, _cx: &CallContext, name: String) -> Result<notes::Note, notes::AccessError> {
        Ok(notes::Note {
            title: name,
            body: Some("contents".into()),
        })
    }

    async fn search(&self, _cx: &CallContext, query: String, limit: u32) -> Vec<notes::Note> {
        std::future::ready(()).await;
        vec![notes::Note {
            title: format!("{query}:{limit}"),
            body: None,
        }]
    }

    fn clear(&self, _cx: &CallContext) {}
}

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

#[test]
fn arc_hosts_forward_plain_and_async_methods() {
    let host = Arc::new(Journal);
    let context = CallContext::for_test("summarizer");
    let note = notes::Host::read(&host, &context, "daily".into()).unwrap();
    assert_eq!(note.title, "daily");
    let found = support::block_on(notes::Host::search(&host, &context, "rust".into(), 3));
    assert_eq!(found[0].title, "rust:3");
}

#[cfg(target_arch = "wasm32")]
const _: () = {
    struct BrowserHost(std::rc::Rc<()>);

    impl notes::Host for BrowserHost {
        fn read(&self, _: &CallContext, _: String) -> Result<notes::Note, notes::AccessError> {
            Err(notes::AccessError::Missing)
        }
        async fn search(&self, _: &CallContext, _: String, _: u32) -> Vec<notes::Note> {
            std::future::ready(()).await;
            let _ = self.0.clone();
            Vec::new()
        }
        fn clear(&self, _: &CallContext) {}
    }
};

use std::sync::{Arc, Mutex};

use keel_sdk::{Event, HookBus};
use serde_json::json;

#[tokio::test]
async fn delivers_in_subscription_order_and_to_wildcards() {
    let bus = HookBus::new();
    let log = Arc::new(Mutex::new(Vec::new()));
    for tag in ["first", "second"] {
        let log = log.clone();
        bus.subscribe("user.created", move |e: Event| {
            let log = log.clone();
            async move { log.lock().unwrap().push(format!("{tag}:{}", e.payload["id"])) }
        });
    }
    let all = log.clone();
    bus.subscribe("*", move |e: Event| {
        let all = all.clone();
        async move { all.lock().unwrap().push(format!("any:{}", e.name)) }
    });

    let ran = bus.emit(Event::new("user.created", json!({ "id": 7 }))).await;
    assert_eq!(ran, 3);
    assert_eq!(
        *log.lock().unwrap(),
        vec!["first:7", "second:7", "any:user.created"]
    );

    let ran = bus.emit(Event::new("order.paid", json!({}))).await;
    assert_eq!(ran, 1);
    assert_eq!(bus.subscriber_count("user.created"), 2);
}

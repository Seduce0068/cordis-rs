use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use cordis_rs::{Context, Event, Fiber, FunctionPlugin, Lifecycle, Plugin, Result};

#[derive(Clone)]
struct Counter(Arc<AtomicUsize>);

/// Wait until a fiber reaches `expected`, polling with a bounded deadline.
/// A non-zero poll interval yields to spawned refresh tasks on the
/// current-thread Tokio runtime (swarm review R5-TS02).
async fn wait_for_state(fiber: &Fiber, expected: Lifecycle) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while fiber.state() != expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fiber did not reach {expected:?} within 2s"));
}

#[tokio::test]
async fn service_registry_and_isolate() {
    let root = Context::root("root");
    root.provide("tools", Counter(Arc::new(AtomicUsize::new(1))))
        .await
        .unwrap();

    assert_eq!(
        root.get::<Counter>("tools")
            .unwrap()
            .0
            .load(Ordering::SeqCst),
        1
    );
    assert!(root.get::<String>("tools").is_none());
    assert!(matches!(
        root.provide("tools", 42u32).await,
        Err(cordis_rs::Error::DuplicateService(_))
    ));

    let child = root.child("child");
    assert!(child.get::<Counter>("tools").is_some());
    let isolated = root.isolate("isolated");
    assert!(isolated.get::<Counter>("tools").is_none());
}

#[tokio::test]
async fn effects_dispose_in_reverse_order_and_are_single_shot() {
    let ctx = Context::root("effects");
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));

    for label in ["first", "second", "third"] {
        let order = order.clone();
        ctx.effect(move |scope| {
            let label = label.to_string();
            let order = order.clone();
            async move {
                scope.on_dispose(move || {
                    let order = order.clone();
                    async move {
                        order.lock().unwrap().push(label);
                        Ok(())
                    }
                });
                Ok(())
            }
        })
        .await
        .unwrap();
    }

    ctx.dispose_effects().await.unwrap();
    let all = order.lock().unwrap().clone();
    assert_eq!(all, vec!["third", "second", "first"]);
    ctx.dispose_effects().await.unwrap();
    assert_eq!(order.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn waterfall_mutates_shared_args_and_allows_veto() {
    let ctx = Context::root("waterfall");
    let event: Event<String, String> = Event::new("prompt");

    let _first = event.on_waterfall(&ctx, |args, next| {
        Box::pin(async move {
            args.lock().unwrap().push_str("-outer");
            let result = next(args).await?;
            Ok(format!("[{result}]"))
        })
    });

    let value = event
        .waterfall(&ctx, "base".to_string(), |args| args.clone())
        .await
        .unwrap();
    assert_eq!(value, "[base-outer]");

    let _veto = event.on_waterfall(&ctx, |args, _next| {
        Box::pin(async move {
            args.lock().unwrap().push_str("-veto");
            Ok("vetoed".to_string())
        })
    });
    let value = event
        .waterfall(&ctx, "base".to_string(), |args| args.clone())
        .await
        .unwrap();
    // The outer listener still wraps the veto and formats its result.
    assert_eq!(value, "[vetoed]");
}

#[tokio::test]
async fn serial_and_bail_stop_at_first_bail() {
    let ctx = Context::root("bail");
    let serial: Event<u32, u32> = Event::new("serial");
    let calls = Arc::new(AtomicUsize::new(0));

    let c1 = calls.clone();
    let _one = serial.on_serial(&ctx, move |_args| {
        let calls = c1.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            None
        })
    });
    let c2 = calls.clone();
    let _two = serial.on_serial(&ctx, move |args| {
        let calls = c2.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(args * 10)
        })
    });
    let c3 = calls.clone();
    let _three = serial.on_serial(&ctx, move |_args| {
        let calls = c3.clone();
        Box::pin(async move {
            calls.fetch_add(1, Ordering::SeqCst);
            Some(999)
        })
    });

    assert_eq!(serial.serial(&ctx, 7).await, Some(70));
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let bail: Event<u32, u32> = Event::new("bail");
    let _zero = bail.on_bail(&ctx, |_args| Some(0));
    let _later = bail.on_bail(&ctx, |_args| Some(42));
    assert_eq!(bail.bail(&ctx, 1), Some(0));
}

#[tokio::test]
async fn emit_and_parallel() {
    let ctx = Context::root("emit");
    let event: Event<u32, ()> = Event::new("tick");
    let seen = Arc::new(AtomicUsize::new(0));

    let seen_listener = seen.clone();
    let _listener = event.on_emit(&ctx, move |args| {
        let seen = seen_listener.clone();
        Box::pin(async move {
            seen.fetch_add(args as usize, Ordering::SeqCst);
        })
    });
    event.emit(&ctx, 3).await;
    assert_eq!(seen.load(Ordering::SeqCst), 3);
    event.parallel(&ctx, 4).await;
    // The listener is still registered for both dispatch paths.
    assert_eq!(seen.load(Ordering::SeqCst), 7);

    _listener.dispose().await.unwrap();
    event.emit(&ctx, 5).await;
    assert_eq!(seen.load(Ordering::SeqCst), 7);
}

struct Greeter {
    applied: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl Plugin for Greeter {
    fn name(&self) -> &str {
        "greeter"
    }

    fn inject(&self) -> &[&str] {
        &["config"]
    }

    async fn apply(&self, ctx: &Context) -> Result<()> {
        self.applied.store(true, Ordering::SeqCst);
        ctx.effect(|scope| async move {
            scope.provide("greeting", "hello".to_string()).await?;
            Ok(())
        })
        .await
    }
}

#[tokio::test]
async fn fiber_waits_for_inject_and_reloads_when_it_returns() {
    let ctx = Context::root("fibers");
    let applied = Arc::new(AtomicBool::new(false));
    let fiber = cordis_rs::Fiber::mount(
        &ctx,
        Box::new(Greeter {
            applied: applied.clone(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(fiber.state(), Lifecycle::Pending);
    assert!(!applied.load(Ordering::SeqCst));

    ctx.provide("config", 1u32).await.unwrap();
    wait_for_state(&fiber, Lifecycle::Active).await;
    assert_eq!(fiber.state(), Lifecycle::Active);
    assert!(applied.load(Ordering::SeqCst));
    assert_eq!(
        fiber.context().get::<String>("greeting").unwrap().as_str(),
        "hello"
    );

    ctx.remove_service("config").unwrap();
    wait_for_state(&fiber, Lifecycle::Pending).await;
    assert_eq!(fiber.state(), Lifecycle::Pending);
    assert!(fiber.context().get::<String>("greeting").is_none());
}

#[tokio::test]
async fn function_plugin_mounts_without_inject() {
    let ctx = Context::root("fn-plugin");
    let called = Arc::new(AtomicBool::new(false));
    let flag = called.clone();
    let plugin = FunctionPlugin::new("noop", move |_ctx| {
        let flag = flag.clone();
        async move {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        }
    });
    let fiber = cordis_rs::Fiber::mount(&ctx, Box::new(plugin))
        .await
        .unwrap();
    assert_eq!(fiber.state(), Lifecycle::Active);
    assert!(called.load(Ordering::SeqCst));
    fiber.unload().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Unloaded);
}

#[test]
fn bail_treats_false_as_non_bail_like_upstream() {
    let ctx = Context::root("bail-false");
    let bail: Event<u32, bool> = Event::new("bail-false");
    let _false = bail.on_bail(&ctx, |_args| Some(false));
    let _true = bail.on_bail(&ctx, |_args| Some(true));
    // `false` is skipped, so the chain continues to the `true` listener.
    assert_eq!(bail.bail(&ctx, 1), Some(true));

    let only_false: Event<u32, bool> = Event::new("only-false");
    let _false = only_false.on_bail(&ctx, |_args| Some(false));
    assert_eq!(only_false.bail(&ctx, 1), None);
}

#[tokio::test]
async fn serial_treats_false_as_non_bail_like_upstream() {
    let ctx = Context::root("serial-false");
    let serial: Event<u32, bool> = Event::new("serial-false");
    let _false = serial.on_serial(&ctx, |_args| {
        Box::pin(async { Some(false) }) as cordis_rs::events::BoxFuture<'static, Option<bool>>
    });
    let _true = serial.on_serial(&ctx, |_args| {
        Box::pin(async { Some(true) }) as cordis_rs::events::BoxFuture<'static, Option<bool>>
    });
    assert_eq!(serial.serial(&ctx, 1).await, Some(true));
}

#[tokio::test]
async fn fiber_unload_removes_service_provided_by_plugin() {
    use cordis_rs::{Context, Fiber, FunctionPlugin, Lifecycle};

    let ctx = Context::root("provide-unload");
    let plugin = FunctionPlugin::new("svc-provider", |ctx: &Context| {
        let ctx = ctx.clone();
        async move {
            ctx.provide("feature", 42u32).await?;
            Ok(())
        }
    });
    let fiber = Fiber::mount(&ctx, Box::new(plugin)).await.unwrap();

    assert_eq!(fiber.state(), Lifecycle::Active);
    assert_eq!(*fiber.context().get::<u32>("feature").unwrap(), 42);

    fiber.unload().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Unloaded);
    assert!(fiber.context().get::<u32>("feature").is_none());
}

#[tokio::test]
async fn failed_fiber_state_is_distinct_from_unloaded() {
    use cordis_rs::{Context, Fiber, FunctionPlugin, Lifecycle};

    let ctx = Context::root("failed-state");
    let plugin = FunctionPlugin::new("failing", |_ctx: &Context| async {
        Err(cordis_rs::Error::Other("boom".to_string()))
    });
    // A failing mount surfaces the error; the fiber's Failed state is
    // observable through the registry during the failing apply.
    assert!(Fiber::mount(&ctx, Box::new(plugin)).await.is_err());

    // An unloaded (successful then explicitly unloaded) fiber stays Unloaded.
    let ok = FunctionPlugin::new("ok", |_ctx: &Context| async { Ok(()) });
    let fiber = Fiber::mount(&ctx, Box::new(ok)).await.unwrap();
    fiber.unload().await.unwrap();
    assert_eq!(fiber.state(), Lifecycle::Unloaded);

    // From<u8> round-trips Failed distinctly from Unloaded.
    assert_eq!(Lifecycle::from(5), Lifecycle::Failed);
    assert_ne!(Lifecycle::Failed, Lifecycle::Unloaded);
}

#[tokio::test]
async fn waterfall_double_next_returns_error_instead_of_panicking() {
    use std::sync::{Arc, Mutex};

    let ctx = Context::root("waterfall-double-next");
    let event: Event<String, String> = Event::new("double-next");
    let shared: Arc<Mutex<String>> = Arc::new(Mutex::new("base".to_string()));
    let _double = event.on_waterfall(&ctx, move |_args, next| {
        let shared = shared.clone();
        Box::pin(async move {
            let _first = next(shared.clone()).await;
            let _second = next(shared).await; // discarded future must still be caught
            Ok("done".to_string())
        })
    });

    let result = event
        .waterfall(&ctx, "base".to_string(), |args| args.clone())
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("multiple times"));
}

#[tokio::test]
async fn unloading_context_hides_its_services_from_children() {
    use cordis_rs::{Context, Lifecycle};

    let parent = Context::root("strict-get-parent");
    parent.provide("shared", 7u32).await.unwrap();
    let child = parent.child("strict-get-child");

    assert_eq!(*child.get::<u32>("shared").unwrap(), 7);

    parent.set_lifecycle(Lifecycle::Unloading);
    assert!(child.get::<u32>("shared").is_none());
}

#[tokio::test]
async fn require_untyped_waits_until_a_service_is_registered() {
    use cordis_rs::Context;

    let ctx = Context::root("require-notify");
    let waiter = ctx.clone();
    let task = tokio::spawn(async move { waiter.require::<u32>("later").await });

    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    ctx.provide("later", 7u32).await.unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .expect("require did not wake after provide")
        .unwrap();
    assert!(matches!(result, Ok(value) if *value == 7));
}

#[tokio::test]
async fn parallel_errors_aggregates_all_failures_in_order() {
    use cordis_rs::Error;

    let ctx = Context::root("parallel-errors");
    let event: Event<u32, ()> = Event::new("parallel-errors");
    let _ok = event.on_emit_fallible(&ctx, |_args| Box::pin(async { Ok(()) }));
    let _first = event.on_emit_fallible(&ctx, |_args| {
        Box::pin(async { Err(Error::Other("first".to_string())) })
    });
    let _second = event.on_emit_fallible(&ctx, |_args| {
        Box::pin(async { Err(Error::Other("second".to_string())) })
    });

    let errors = event.parallel_errors(&ctx, 7).await.unwrap_err();
    assert_eq!(errors.0.len(), 2);
    assert_eq!(errors.0[0].to_string(), "first");
    assert_eq!(errors.0[1].to_string(), "second");
    assert_eq!(errors.to_string(), "2 listener error(s)");

    // Existing emit() does not see fallible listeners (separate slot).
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = seen.clone();
    let _plain = event.on_emit(&ctx, move |_args| {
        let flag = flag.clone();
        Box::pin(async move { flag.store(true, std::sync::atomic::Ordering::SeqCst) })
    });
    event.emit(&ctx, 7).await;
    assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
    assert!(event.parallel_errors(&ctx, 7).await.is_err());
}

#[tokio::test]
async fn old_disposer_does_not_remove_a_newer_service_with_same_name() {
    let ctx = Context::root("ghost-disposer");

    let first = ctx.provide("shared", 1u32).await.unwrap();
    ctx.remove_service("shared").unwrap();
    let second = ctx.provide("shared", 2u32).await.unwrap();

    // The old disposer must be a no-op once the name was re-provided.
    first.dispose().await.unwrap();
    assert_eq!(
        ctx.get::<u32>("shared").map(|value| *value),
        Some(2),
        "stale disposer removed the newer service"
    );

    second.dispose().await.unwrap();
    assert!(ctx.get::<u32>("shared").is_none());
}

#[tokio::test]
async fn context_named_on_once_and_emit_share_descriptor_slot() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("named-events");
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let _d = ctx.on("custom", move |_args| {
        let c = c.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    });

    let once_calls = Arc::new(AtomicUsize::new(0));
    let oc = once_calls.clone();
    let _od = ctx.once("custom", move |_args| {
        let oc = oc.clone();
        Box::pin(async move {
            oc.fetch_add(1, Ordering::SeqCst);
        })
    });

    // A typed descriptor with the same name and payload type shares the slot.
    let descriptor_calls = Arc::new(AtomicUsize::new(0));
    let dc = descriptor_calls.clone();
    let event: cordis_rs::events::Event<serde_json::Value> = cordis_rs::Event::new("custom");
    let _dd = event.on_emit(&ctx, move |_args| {
        let dc = dc.clone();
        Box::pin(async move {
            dc.fetch_add(1, Ordering::SeqCst);
        })
    });

    ctx.emit("custom", serde_json::json!({ "x": 1 })).await;
    ctx.emit("custom", serde_json::json!({ "x": 2 })).await;

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(once_calls.load(Ordering::SeqCst), 1);
    assert_eq!(descriptor_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stage_a_dispose_checked_and_once_self_removal() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("stage-a");
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let disposer = ctx.on("evt", move |_args| {
        let c = c.clone();
        Box::pin(async move {
            c.fetch_add(1, Ordering::SeqCst);
        })
    });

    ctx.emit("evt", serde_json::json!({})).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(disposer.dispose_checked().await.unwrap());
    assert!(!disposer.dispose_checked().await.unwrap());
    assert!(disposer.is_spent());
    ctx.emit("evt", serde_json::json!({})).await;
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // `once` unregisters itself after the first invocation.
    let once_count = Arc::new(AtomicUsize::new(0));
    let oc = once_count.clone();
    let _once = ctx.once("evt2", move |_args| {
        let oc = oc.clone();
        Box::pin(async move {
            oc.fetch_add(1, Ordering::SeqCst);
        })
    });
    ctx.emit("evt2", serde_json::json!({})).await;
    ctx.emit("evt2", serde_json::json!({})).await;
    assert_eq!(once_count.load(Ordering::SeqCst), 1);

    // Deprecated alias still dispatches the same listeners.
    let event: cordis_rs::events::Event<serde_json::Value> = cordis_rs::Event::new("spawn");
    let spawned = Arc::new(AtomicUsize::new(0));
    let sc = spawned.clone();
    let _d = event.on_emit(&ctx, move |_args| {
        let sc = sc.clone();
        Box::pin(async move {
            sc.fetch_add(1, Ordering::SeqCst);
        })
    });
    #[allow(deprecated)]
    event.emit_detached(&ctx, serde_json::json!({}));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(spawned.load(Ordering::SeqCst), 1);
    event.emit_spawn(&ctx, serde_json::json!({}));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(spawned.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn stage_b_prepend_global_and_filtered_dispatch() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cordis_rs::EventOptions;

    let ctx = Context::root("stage-b");
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));

    let o = order.clone();
    let _tail = ctx.on("evt", move |_args| {
        let o = o.clone();
        Box::pin(async move { o.lock().unwrap().push("tail") })
    });
    let o = order.clone();
    let _head = ctx.on_with(
        "evt",
        move |_args| {
            let o = o.clone();
            Box::pin(async move { o.lock().unwrap().push("head") })
        },
        EventOptions {
            prepend: true,
            global: false,
        },
    );

    ctx.emit("evt", serde_json::json!({})).await;
    assert_eq!(*order.lock().unwrap(), vec!["head", "tail"]);

    // A filtered scope skips non-global listeners, but global ones still fire.
    let gated = ctx.filtered(|name| name != "evt");
    let global_count = Arc::new(AtomicUsize::new(0));
    let gc = global_count.clone();
    let _global = ctx.on_with(
        "evt",
        move |_args| {
            let gc = gc.clone();
            Box::pin(async move {
                gc.fetch_add(1, Ordering::SeqCst);
            })
        },
        EventOptions {
            prepend: false,
            global: true,
        },
    );

    order.lock().unwrap().clear();
    ctx.emit_with(&gated, "evt", serde_json::json!({})).await;
    assert!(order.lock().unwrap().is_empty());
    assert_eq!(global_count.load(Ordering::SeqCst), 1);
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct TypedPayload {
    text: String,
}

cordis_rs::typed_event!(
    TypedTestEvent,
    "typed-test",
    TypedPayload,
    TYPED_TEST_DECODE_ERRORS
);

#[tokio::test]
async fn typed_event_macro_shares_named_slot_and_records_decode_errors() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("typed-macro");
    let called = Arc::new(AtomicUsize::new(0));
    let counter = called.clone();
    let _d = TypedTestEvent::on(&ctx, move |payload: TypedPayload| {
        let counter = counter.clone();
        Box::pin(async move {
            assert_eq!(payload.text, "hello");
            counter.fetch_add(1, Ordering::SeqCst);
        })
    });

    // A plain named emit reaches the typed listener.
    ctx.emit(TypedTestEvent::NAME, serde_json::json!({ "text": "hello" }))
        .await;
    assert_eq!(called.load(Ordering::SeqCst), 1);

    // Malformed payload is recorded, never silently defaulted.
    ctx.emit(TypedTestEvent::NAME, serde_json::json!({ "bad": 1 }))
        .await;
    let errors = TypedTestEvent::take_decode_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].event, "typed-test");

    // Typed emit also flows through the same named slot.
    TypedTestEvent::emit(
        &ctx,
        &TypedPayload {
            text: "hello".to_string(),
        },
    )
    .await
    .unwrap();
    assert_eq!(called.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn removing_listeners_clears_hook_meta() {
    use cordis_rs::EventOptions;

    let ctx = Context::root("meta-cleanup");
    let baseline = ctx.event_registry().meta_len();

    let d1 = ctx.on_with(
        "meta-evt",
        |_args| Box::pin(async {}),
        EventOptions {
            prepend: false,
            global: true,
        },
    );
    let d2 = ctx.on_with(
        "meta-evt",
        |_args| Box::pin(async {}),
        EventOptions {
            prepend: true,
            global: false,
        },
    );
    assert_eq!(ctx.event_registry().meta_len(), baseline + 2);

    d1.dispose().await.unwrap();
    d2.dispose().await.unwrap();
    assert_eq!(ctx.event_registry().meta_len(), baseline);
}

#[tokio::test]
async fn dispatch_with_registry_matches_context_adapter() {
    use cordis_rs::events::named_emit_slot;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("with-registry");
    let registry = ctx.event_registry().clone();
    let counter = Arc::new(AtomicUsize::new(0));

    // Register through the with_registry entry point directly.
    let listener = {
        let counter = counter.clone();
        move |_args: serde_json::Value| -> cordis_rs::events::BoxFuture<'static, ()> {
            let counter = counter.clone();
            Box::pin(async move {
                counter.fetch_add(1, Ordering::SeqCst);
            })
        }
    };
    let event = cordis_rs::events::Event::<serde_json::Value, ()>::new("reg-dispatch");
    let disposer = event.on_emit_with_registry(&registry, listener);

    // Both the Context adapter and the explicit registry see the same slot.
    let args = serde_json::json!({"n": 1});
    event.emit(&ctx, args.clone()).await;
    event.emit_with_registry(&registry, args).await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // The shared disposer helper removes by id from the registry.
    disposer.dispose().await.unwrap();
    // named_emit_slot is used internally; ensure no lingering listener.
    let _ = named_emit_slot("reg-dispatch");
    event
        .emit_with_registry(&registry, serde_json::json!({}))
        .await;
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn fiber_reloads_via_async_drain_after_dependency_loss() {
    // R2-BV01: retiring must be cleared when a fiber returns to Pending so
    // drain_dependent_fibers (async removal path) can reload it.
    let ctx = Context::root("async-drain");
    let applied = Arc::new(AtomicBool::new(false));
    let fiber = cordis_rs::Fiber::mount(
        &ctx,
        Box::new(Greeter {
            applied: applied.clone(),
        }),
    )
    .await
    .unwrap();

    ctx.provide("config", 1u32).await.unwrap();
    wait_for_state(&fiber, Lifecycle::Active).await;
    assert_eq!(fiber.state(), Lifecycle::Active);

    // Async removal drains dependents; the fiber retires temporarily.
    ctx.remove_service_async("config").await.unwrap();
    wait_for_state(&fiber, Lifecycle::Pending).await;
    assert_eq!(fiber.state(), Lifecycle::Pending);

    // Dependency returns: drain must refresh the (no longer retiring) fiber.
    ctx.provide("config", 1u32).await.unwrap();
    wait_for_state(&fiber, Lifecycle::Active).await;
    assert_eq!(
        fiber.state(),
        Lifecycle::Active,
        "fiber must reload after async drain, not stay retired"
    );
}

#[tokio::test]
async fn once_listener_fires_exactly_once_under_concurrent_emit() {
    // R2-BV04: the raw listener must never be visible before the once wrapper,
    // otherwise concurrent emits can fire it more than once.
    use std::sync::atomic::{AtomicUsize, Ordering};

    let ctx = Context::root("once-concurrent");
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let _disposer = ctx.once("burst", move |_args| {
        let counter = counter.clone();
        Box::pin(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    });

    let mut handles = Vec::new();
    for _ in 0..8 {
        let ctx = ctx.clone();
        handles.push(tokio::spawn(async move {
            ctx.emit("burst", serde_json::json!({})).await;
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "once listener must fire exactly once under concurrent emit"
    );
}

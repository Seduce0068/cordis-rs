use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use cordis_rs::{Context, Event, FunctionPlugin, Lifecycle, Plugin, Result};

#[derive(Clone)]
struct Counter(Arc<AtomicUsize>);

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
            let result = next(args).await;
            format!("[{result}]")
        })
    });

    let value = event
        .waterfall(&ctx, "base".to_string(), |args| args.clone())
        .await;
    assert_eq!(value, "[base-outer]");

    let _veto = event.on_waterfall(&ctx, |args, _next| {
        Box::pin(async move {
            args.lock().unwrap().push_str("-veto");
            "vetoed".to_string()
        })
    });
    let value = event
        .waterfall(&ctx, "base".to_string(), |args| args.clone())
        .await;
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
        ctx.effect(|scope| async move { scope.provide("greeting", "hello".to_string()).await })
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
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(fiber.state(), Lifecycle::Active);
    assert!(applied.load(Ordering::SeqCst));
    assert_eq!(
        fiber.context().get::<String>("greeting").unwrap().as_str(),
        "hello"
    );

    ctx.remove_service("config").unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
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

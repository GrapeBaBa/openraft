use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fixtures::RaftRouter;
use maplit::btreeset;
use openraft::Config;
use openraft::RaftStorage;
use openraft::State;
use tokio::sync::watch;

#[macro_use]
mod fixtures;

/// The logs have to be applied in log index order.
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn total_order_apply() -> Result<()> {
    let (_log_guard, ut_span) = init_ut!();
    let _ent = ut_span.enter();

    // Setup test dependencies.
    let config = Arc::new(Config::default().validate().expect("failed to build Raft config"));
    let router = Arc::new(RaftRouter::new(config.clone()));

    router.new_raft_node(0).await;
    router.new_raft_node(1).await;

    tracing::info!("--- initializing single node cluster");

    router.initialize_with(0, btreeset![0]).await?;
    router
        .wait_for_metrics(&0u64, |x| x.state == State::Leader, timeout(), "n0.state -> Leader")
        .await?;

    tracing::info!("--- add one learner");
    router.add_learner(0, 1).await?;

    let (tx, rx) = watch::channel(false);

    let sto1 = router.get_storage_handle(&1).await?;

    let mut prev = 0;
    let h = tokio::spawn(async move {
        loop {
            if *rx.borrow() {
                break;
            }

            let (last, _) = sto1.last_applied_state().await.unwrap();

            if last.index < prev {
                panic!("out of order apply");
            }
            prev = last.index;
        }
    });

    let n = 10_000;
    router.client_request_many(0, "foo", n).await;

    // stop the log checking task.
    tx.send(true)?;
    h.await?;

    let want = n as u64;
    router
        .wait_for_metrics(
            &1u64,
            |x| x.last_applied >= want,
            timeout(),
            &format!("n{}.last_applied -> {}", 1, want),
        )
        .await?;

    Ok(())
}

fn timeout() -> Option<Duration> {
    Some(Duration::from_millis(2000))
}

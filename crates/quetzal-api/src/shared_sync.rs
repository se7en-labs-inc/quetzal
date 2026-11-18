//! Fixed live/catch-up followers. Wallets share decoded blocks and retain their
//! own recoverable cursors. All replay/database work runs on the owned thread.
use crate::config::Config;
use quetzal_network::{chainsync::ChainSyncDriver, ChainSyncEvent, NodeClient};
use quetzal_primitives::Checkpoint;
use quetzal_wallet::{ChainSyncWorker, WalletError};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const MAX_WALLETS: usize = 256;
const COHORT_SIZE: usize = 64;
const QUANTUM: usize = 4096;

enum Command {
    Add {
        worker: Box<ChainSyncWorker>,
        reply: oneshot::Sender<Result<u64, WalletError>>,
    },
    Remove {
        id: u64,
        reply: oneshot::Sender<()>,
    },
}
struct Member {
    active: AtomicBool,
    worker: Box<ChainSyncWorker>,
    cursor: Mutex<Checkpoint>,
}
impl Member {
    fn point(&self) -> Result<Checkpoint, WalletError> {
        self.cursor
            .lock()
            .map(|point| point.clone())
            .map_err(|_| WalletError::TxBuild("sync cursor lock poisoned".into()))
    }
}
struct Completed {
    live: bool,
    caught_up: bool,
    fatal: bool,
    members: Vec<Arc<Member>>,
}

#[derive(Default)]
struct Counters {
    decoded: AtomicU64,
    applied: AtomicU64,
    received_bytes: AtomicU64,
    pool_blocks: AtomicU64,
}

pub(crate) struct SharedSync {
    pub(crate) failed: CancellationToken,
    send: mpsc::Sender<Command>,
    stop: CancellationToken,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    counters: Arc<Counters>,
}
impl SharedSync {
    pub(crate) fn new(config: Config) -> Result<Self, WalletError> {
        let (send, receive) = mpsc::channel(64);
        let stop = CancellationToken::new();
        let cancelled = stop.clone();
        let failed = CancellationToken::new();
        let failure_signal = failed.clone();
        let decoded = Arc::new(Counters::default());
        let counter = decoded.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| WalletError::TxBuild(format!("sync runtime: {error}")))?;
        let thread = std::thread::Builder::new()
            .name("wallet-chain-sync".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    runtime.block_on(coordinate(config, receive, cancelled, counter))
                }));
                if !matches!(result, Ok(false)) {
                    failure_signal.cancel();
                }
            })
            .map_err(|error| WalletError::TxBuild(format!("sync thread: {error}")))?;
        Ok(Self {
            failed,
            send,
            stop,
            thread: Mutex::new(Some(thread)),
            counters: decoded,
        })
    }
    pub(crate) async fn register(&self, worker: ChainSyncWorker) -> Result<u64, WalletError> {
        let (reply, response) = oneshot::channel();
        self.send
            .send(Command::Add {
                worker: Box::new(worker),
                reply,
            })
            .await
            .map_err(|_| WalletError::TxBuild("sync service closed".into()))?;
        response
            .await
            .map_err(|_| WalletError::TxBuild("sync registration interrupted".into()))?
    }
    pub(crate) async fn remove(&self, id: u64) {
        let (reply, response) = oneshot::channel();
        if self.send.send(Command::Remove { id, reply }).await.is_ok() {
            let _ = response.await;
        }
    }
    pub(crate) async fn shutdown(&self) -> anyhow::Result<()> {
        self.stop.cancel();
        let thread = self
            .thread
            .lock()
            .map_err(|_| anyhow::anyhow!("sync join lock poisoned"))?
            .take();
        if let Some(thread) = thread {
            tokio::task::spawn_blocking(move || thread.join())
                .await?
                .map_err(|_| anyhow::anyhow!("sync thread panicked"))?;
        }
        tracing::info!(
            decoded_blocks = self.counters.decoded.load(Ordering::Relaxed),
            wallet_blocks = self.counters.applied.load(Ordering::Relaxed),
            received_bytes = self.counters.received_bytes.load(Ordering::Relaxed),
            pool_blocks = self.counters.pool_blocks.load(Ordering::Relaxed),
            "Shared wallet sync stopped"
        );
        Ok(())
    }
}
impl Drop for SharedSync {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Ok(thread) = self.thread.get_mut() {
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}

async fn coordinate(
    config: Config,
    mut receive: mpsc::Receiver<Command>,
    stop: CancellationToken,
    decoded: Arc<Counters>,
) -> bool {
    let mut members = HashMap::<u64, Arc<Member>>::new();
    let mut pending = VecDeque::<(bool, Vec<Arc<Member>>)>::new();
    let mut live_members = Vec::<Arc<Member>>::new();
    let mut tasks = JoinSet::<Completed>::new();
    let mut live_stop: Option<CancellationToken> = None;
    let mut catchup_stop: Option<CancellationToken> = None;
    let mut next_id = 0u64;
    let mut last_report = Instant::now();
    let mut catchup_ready = Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            _ = tokio::time::sleep_until(catchup_ready.into()), if catchup_stop.is_none() && !pending.is_empty() => {},
            command = receive.recv() => match command {
                None => break,
                Some(Command::Add { worker, reply }) => {
                    if members.len() >= MAX_WALLETS {
                        let _ = reply.send(Err(WalletError::TxBuild("sync wallet capacity exhausted".into())));
                        continue;
                    }
                    let point = match worker.checkpoints() {
                        Ok(points) => points.into_iter().next().unwrap_or_else(Checkpoint::origin),
                        Err(error) => { let _ = reply.send(Err(error)); continue; }
                    };
                    if catchup_stop.is_none() && pending.is_empty() {
                        catchup_ready = Instant::now() + Duration::from_millis(20);
                    }
                    next_id += 1;
                    let member = Arc::new(Member { active: AtomicBool::new(true), worker, cursor: Mutex::new(point) });
                    // Pending registrations share a cohort; active cohorts keep
                    // their quantum so continual joins cannot rewind them.
                    if let Some((_, group)) = pending.back_mut().filter(|(started, group)| !started && group.len() < COHORT_SIZE) {
                        group.push(member.clone());
                    } else { pending.push_back((false, vec![member.clone()])); }
                    members.insert(next_id, member);
                    let _ = reply.send(Ok(next_id));
                }
                Some(Command::Remove { id, reply }) => {
                    if let Some(member) = members.remove(&id) { member.active.store(false, Ordering::Release); }
                    if let Some(token) = &live_stop { token.cancel(); }
                    if let Some(token) = &catchup_stop { token.cancel(); }
                    // Replay and this command execute on the same thread. No
                    // removed member can apply a block after this acknowledgment.
                    let _ = reply.send(());
                }
            },
            result = tasks.join_next(), if !tasks.is_empty() => match result {
                Some(Ok(mut result)) => {
                    if result.fatal { break; }
                    result.members.retain(|member| member.active.load(Ordering::Acquire));
                    if result.live { live_stop = None; }
                    else { catchup_stop = None; }
                    if result.caught_up || result.live {
                        if !result.live { live_members.extend(result.members); }
                        if let Some(token) = &live_stop { token.cancel(); }
                    } else if !result.members.is_empty() { pending.push_back((true, result.members)); }
                }
                Some(Err(error)) => { tracing::error!(%error,"Shared sync task panicked"); break; }
                None => {},
            },
        }
        live_members.retain(|member| member.active.load(Ordering::Acquire));
        if last_report.elapsed() >= Duration::from_secs(10) {
            tracing::info!(
                wallets = members.len(),
                live_wallets = live_members.len(),
                catchup_groups = pending.len(),
                decoded_blocks = decoded.decoded.load(Ordering::Relaxed),
                wallet_blocks = decoded.applied.load(Ordering::Relaxed),
                received_bytes = decoded.received_bytes.load(Ordering::Relaxed),
                "Shared wallet sync progress"
            );
            last_report = Instant::now();
        }
        if live_stop.is_none() && !live_members.is_empty() {
            let token = stop.child_token();
            tasks.spawn(follow(
                config.clone(),
                live_members.clone(),
                true,
                token.clone(),
                decoded.clone(),
            ));
            live_stop = Some(token);
        }
        if catchup_stop.is_none() && Instant::now() >= catchup_ready {
            while let Some((_, mut group)) = pending.pop_front() {
                group.retain(|member| member.active.load(Ordering::Acquire));
                if group.is_empty() {
                    continue;
                }
                let token = stop.child_token();
                tasks.spawn(follow(
                    config.clone(),
                    group,
                    false,
                    token.clone(),
                    decoded.clone(),
                ));
                catchup_stop = Some(token);
                break;
            }
        }
    }
    let failed = !stop.is_cancelled();
    stop.cancel();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            tracing::error!(%error,"Shared sync join failed");
        }
    }
    failed
}

async fn follow(
    config: Config,
    members: Vec<Arc<Member>>,
    live: bool,
    stop: CancellationToken,
    decoded: Arc<Counters>,
) -> Completed {
    let mut caught_up = false;
    let mut fatal = false;
    let result = async {
        // Reconcile each independent cursor at connection time before sharing a
        // range. Older members are never fast-forwarded to another wallet's tip.
        let mut points = Vec::new();
        for member in &members {
            let point = member.point()?;
            points.push(point);
        }
        points.sort_by_key(|point| point.slot);
        let start = points.first().cloned().unwrap_or_else(Checkpoint::origin);
        let mut node = tokio::select! {
            biased;
            _ = stop.cancelled() => return Ok(()),
            result = NodeClient::connect(&config.socket_path, config.network_magic) => result.map_err(|error| WalletError::TxBuild(error.to_string()))?,
        };
        let work =
            async {
                // Members ahead of the cohort's oldest cursor may have been on a
                // fork while this cohort was queued. Validate before skipping history.
                for member in &members {
                    if !member.active.load(Ordering::Acquire) {
                        continue;
                    }
                    let point = member.point()?;
                    if point.slot > start.slot
                        && !NodeClient::validate_checkpoint(
                            &config.socket_path,
                            config.network_magic,
                            &point,
                        )
                        .await
                        .map_err(|error| WalletError::TxBuild(error.to_string()))?
                    {
                        if !member.active.load(Ordering::Acquire) {
                            continue;
                        }
                        let origin = Checkpoint::origin();
                        member.worker.process_intersection(&origin).await?;
                        *member.cursor.lock().map_err(|_| {
                            WalletError::TxBuild("sync cursor lock poisoned".into())
                        })? = origin;
                        return Ok(());
                    }
                }
                let mut driver = ChainSyncDriver::new(node.chainsync());
                let selected = driver
                    .intersection(std::slice::from_ref(&start))
                    .await
                    .map_err(|error| WalletError::TxBuild(error.to_string()))?;
                // If the requested point no longer exists, every participating
                // wallet must undo back to the actual selected ancestor.
                if selected.slot < start.slot || selected.block_hash != start.block_hash {
                    for member in &members {
                        if !member.active.load(Ordering::Acquire) {
                            continue;
                        }
                        member.worker.process_intersection(&selected).await?;
                        *member.cursor.lock().map_err(|_| {
                            WalletError::TxBuild("sync cursor lock poisoned".into())
                        })? = selected.clone();
                    }
                }
                let began = Instant::now();
                let mut initial = true;
                for count in 0usize.. {
                    match driver
                        .next()
                        .await
                        .map_err(|error| WalletError::TxBuild(error.to_string()))?
                    {
                        ChainSyncEvent::Await => {
                            for member in &members {
                                if member.active.load(Ordering::Acquire) {
                                    member.worker.process_await()?;
                                }
                            }
                            if !live {
                                caught_up = true;
                                break;
                            }
                        }
                        ChainSyncEvent::RollForward {
                            block_cbor,
                            tip_slot,
                        } => {
                            if block_cbor.len() > 4 * 1024 * 1024 {
                                return Err(WalletError::TxBuild(
                                    "sync block exceeds byte budget".into(),
                                ));
                            }
                            let block = pallas_traverse::MultiEraBlock::decode(&block_cbor)
                                .map_err(|error| WalletError::TxBuild(error.to_string()))?;
                            decoded.decoded.fetch_add(1, Ordering::Relaxed);
                            decoded
                                .received_bytes
                                .fetch_add(block_cbor.len() as u64, Ordering::Relaxed);
                            let index = if members
                                .iter()
                                .all(|member| member.worker.is_pool_follower())
                            {
                                quetzal_wallet::sync::BlockIndex::default()
                            } else {
                                quetzal_wallet::sync::BlockIndex::new(&block)
                            };
                            for member in &members {
                                if !member.active.load(Ordering::Acquire) {
                                    continue;
                                }
                                let point = member.point()?;
                                if point != Checkpoint::origin() && block.slot() <= point.slot {
                                    continue;
                                }
                                if let Err(error) =
                                    member.worker.process_indexed(&block, &index, tip_slot)
                                {
                                    fatal = true;
                                    return Err(error);
                                }
                                if member.worker.is_pool_follower() {
                                    decoded.pool_blocks.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    decoded.applied.fetch_add(1, Ordering::Relaxed);
                                }
                                *member.cursor.lock().map_err(|_| {
                                    WalletError::TxBuild("sync cursor lock poisoned".into())
                                })? = Checkpoint {
                                    slot: block.slot(),
                                    block_hash: *block.hash(),
                                    block_height: block.number(),
                                };
                            }
                        }
                        ChainSyncEvent::RollBackward { to_slot, to_hash } => {
                            // ChainSync sends the selected intersection as its
                            // initial rollback. It is not a new fork, and ahead
                            // cohort members have already been validated.
                            if initial && to_slot == selected.slot && to_hash == selected.block_hash
                            {
                                initial = false;
                                continue;
                            }
                            for member in &members {
                                if !member.active.load(Ordering::Acquire) {
                                    continue;
                                }
                                member
                                    .worker
                                    .process_roll_backward(to_slot, to_hash)
                                    .await?;
                                let point = member
                                    .worker
                                    .checkpoints()?
                                    .into_iter()
                                    .next()
                                    .unwrap_or_else(Checkpoint::origin);
                                *member.cursor.lock().map_err(|_| {
                                    WalletError::TxBuild("sync cursor lock poisoned".into())
                                })? = point;
                            }
                            // Restart intersection at a recoverable known point.
                            break;
                        }
                    }
                    initial = false;
                    if !live && (count + 1 >= QUANTUM || began.elapsed() >= Duration::from_secs(2))
                    {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
                Ok(())
            };
        let work = tokio::select! {
            biased;
            _ = stop.cancelled() => Ok(()),
            result = work => result,
        };
        node.abort().await;
        work
    };
    let outcome = result.await;
    for member in &members {
        if member.active.load(Ordering::Acquire) {
            if let Err(error) = member.worker.commit_progress() {
                fatal = true;
                tracing::error!(%error,"Sync progress commit failed");
            }
        }
    }
    if let Err(error) = outcome {
        tracing::warn!(%error,live,"Shared sync reconnecting");
        // Discard speculative state/cursors on reconnect, including a failed
        // commit. A new worker is not required: intersection invalidates caches.
        for member in &members {
            if !member.active.load(Ordering::Acquire) {
                continue;
            }
            if let Ok(points) = member.worker.checkpoints() {
                if let Ok(mut cursor) = member.cursor.lock() {
                    *cursor = points.into_iter().next().unwrap_or_else(Checkpoint::origin);
                }
            }
        }
        tokio::select! { _=stop.cancelled()=>{}, _=tokio::time::sleep(Duration::from_secs(2))=>{} }
    }
    Completed {
        live,
        caught_up,
        fatal,
        members,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use pallas_network::{
        facades::NodeServer,
        miniprotocols::{
            chainsync::{ClientRequest, Tip},
            Point,
        },
    };
    fn empty_block() -> Vec<u8> {
        use pallas_codec::utils::{KeyValuePairs, MaybeIndefArray};
        use pallas_primitives::{babbage, conway, VrfCert};
        let header = babbage::Header {
            header_body: babbage::HeaderBody {
                block_number: 1,
                slot: 1,
                prev_hash: None,
                issuer_vkey: vec![0; 32].into(),
                vrf_vkey: vec![0; 32].into(),
                vrf_result: VrfCert(vec![0; 32].into(), vec![0; 80].into()),
                block_body_size: 0,
                block_body_hash: [0; 32].into(),
                operational_cert: babbage::OperationalCert {
                    operational_cert_hot_vkey: vec![0; 32].into(),
                    operational_cert_sequence_number: 0,
                    operational_cert_kes_period: 0,
                    operational_cert_sigma: vec![0; 64].into(),
                },
                protocol_version: (9, 0),
            },
            body_signature: vec![0; 448].into(),
        };
        let body = conway::Block {
            header,
            transaction_bodies: MaybeIndefArray::Def(vec![]),
            transaction_witness_sets: MaybeIndefArray::Def(vec![]),
            auxiliary_data_set: KeyValuePairs::Def(vec![]),
            invalid_transactions: Some(MaybeIndefArray::Def(vec![])),
        };
        pallas_codec::minicbor::to_vec((7u16, body)).unwrap()
    }
    #[tokio::test]
    async fn initial_intersection_rollback_is_followed_by_shared_block_and_shutdown_closes_socket()
    {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("node.socket");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let service =
            quetzal_wallet::WalletService::new(directory.path().join("db").to_str().unwrap())
                .unwrap();
        let mut members = Vec::new();
        for entropy in [39, 40] {
            let wallet = service
                .create_wallet(
                    "cohort".into(),
                    &[entropy; 20],
                    b"public-test-password",
                    2,
                    pallas_addresses::Network::Testnet,
                )
                .await
                .unwrap();
            members.push(Arc::new(Member {
                active: AtomicBool::new(true),
                worker: Box::new(service.wallet_sync_worker(wallet.id)),
                cursor: Mutex::new(Checkpoint::origin()),
            }));
        }
        let block = empty_block();
        let parsed = pallas_traverse::MultiEraBlock::decode(&block).unwrap();
        let tip = Tip(Point::Specific(1, parsed.hash().to_vec()), 1);
        let server = tokio::spawn(async move {
            let mut node = NodeServer::accept(&listener, 2).await.unwrap();
            assert!(matches!(
                node.chainsync.recv_while_idle().await.unwrap(),
                Some(ClientRequest::Intersect(_))
            ));
            node.chainsync
                .send_intersect_found(Point::Origin, tip.clone())
                .await
                .unwrap();
            assert!(matches!(
                node.chainsync.recv_while_idle().await.unwrap(),
                Some(ClientRequest::RequestNext)
            ));
            node.chainsync
                .send_roll_backward(Point::Origin, tip.clone())
                .await
                .unwrap();
            assert!(matches!(
                node.chainsync.recv_while_idle().await.unwrap(),
                Some(ClientRequest::RequestNext)
            ));
            node.chainsync
                .send_roll_forward(
                    pallas_network::miniprotocols::chainsync::BlockContent(block),
                    tip,
                )
                .await
                .unwrap();
            assert!(matches!(
                node.chainsync.recv_while_idle().await.unwrap(),
                Some(ClientRequest::RequestNext)
            ));
            node.chainsync.send_await_reply().await.unwrap();
            // Keep the peer alive until follow has consumed AwaitReply.
            tokio::time::sleep(Duration::from_millis(100)).await;
            node.abort().await;
        });
        let config = Config {
            socket_path: socket.to_str().unwrap().into(),
            network_magic: 2,
            ..Default::default()
        };
        let counters = Arc::new(Counters::default());
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            follow(
                config,
                members.clone(),
                false,
                CancellationToken::new(),
                counters.clone(),
            ),
        )
        .await
        .unwrap();
        assert!(result.caught_up);
        assert_eq!(counters.decoded.load(Ordering::Relaxed), 1);
        assert_eq!(counters.applied.load(Ordering::Relaxed), 2);
        for member in members {
            assert_eq!(member.worker.checkpoints().unwrap()[0].slot, 1);
        }
        server.await.unwrap();
    }
}

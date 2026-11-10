//! Own the real service and its stdin. Kill/reap is failure cleanup only.
#![cfg(unix)]
#[path = "../src/test_pki.rs"]
mod test_pki;

use reqwest::{Certificate, Client, Identity, Method};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::{Duration, Instant},
};

struct Service {
    child: Child,
    input: Option<ChildStdin>,
    socket: SocketAddr,
    ui_socket: SocketAddr,
    url: String,
    log: PathBuf,
}

impl Drop for Service {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

impl Service {
    fn start(root: &Path, label: &str, tls: bool, flag: bool, pipe: bool) -> Self {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = reservation.local_addr().unwrap();
        let ui_reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let ui_socket = ui_reservation.local_addr().unwrap();
        drop(reservation);
        drop(ui_reservation);
        let log = root.join(format!("{label}.log"));
        let output = File::create(&log).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_quetzal"));
        command
            .arg("serve")
            .arg("--mainnet")
            .arg("--database")
            .arg(root.join("wallet"))
            .arg("--node-socket")
            .arg(root.join("absent-node.socket"))
            .arg("--port")
            .arg(socket.port().to_string())
            .args([
                "--ui-port",
                &ui_socket.port().to_string(),
                "--listen-address",
                "127.0.0.1",
                "--sync-tolerance",
                "42s",
                "--pool-metadata-fetching",
                "none",
                "--token-metadata-server",
                "http://127.0.0.1:1/",
                "--log-level",
                "debug",
                "--trace-api-server",
                "warning",
                "--ipfs-gateway-url",
                "https://ipfs.example/ipfs/",
            ])
            .stdin(if pipe { Stdio::piped() } else { Stdio::null() })
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        if flag {
            command.arg("--shutdown-handler");
        }
        if tls {
            command
                .arg("--tls-ca-cert")
                .arg(root.join("ca.crt"))
                .arg("--tls-sv-cert")
                .arg(root.join("server.crt"))
                .arg("--tls-sv-key")
                .arg(root.join("server.key"));
        }
        let mut child = command.spawn().unwrap();
        let input = child.stdin.take();
        Self {
            child,
            input,
            socket,
            ui_socket,
            url: format!("{}://{socket}/v2", if tls { "https" } else { "http" }),
            log,
        }
    }

    async fn ready(&mut self, client: &Client) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "service exited before readiness: {}",
                fs::read_to_string(&self.log).unwrap()
            );
            if client
                .get(format!("{}/wallets", self.url))
                .send()
                .await
                .is_ok_and(|response| response.status() == 200)
            {
                let scheme = if self.url.starts_with("https:") {
                    "https"
                } else {
                    "http"
                };
                let ui = client
                    .get(format!("{scheme}://{}/", self.ui_socket))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(ui.status(), 200);
                assert!(ui.text().await.unwrap().contains("Quetzal wallets"));
                return;
            }
            assert!(Instant::now() < deadline, "service readiness timed out");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn exited_cleanly(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "unclean exit {status}: {}",
                    fs::read_to_string(&self.log).unwrap()
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "shutdown did not finish within 30 seconds"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            tokio::net::TcpStream::connect(self.socket).await.is_err(),
            "listener survived shutdown"
        );
        let listener = tokio::net::TcpListener::bind(self.socket)
            .await
            .expect("listener was not released");
        drop(listener);
        assert!(
            tokio::net::TcpStream::connect(self.ui_socket)
                .await
                .is_err(),
            "UI listener survived shutdown"
        );
        drop(
            tokio::net::TcpListener::bind(self.ui_socket)
                .await
                .expect("UI listener was not released"),
        );
        assert!(fs::read_to_string(&self.log)
            .unwrap()
            .contains("Quetzal shutdown complete"));
    }

    fn eof(&mut self) {
        drop(self.input.take());
    }

    fn terminate(&self) {
        // SAFETY: the Child handle owns this still-live process; it has not been
        // reaped or released, so the PID cannot be reused for an unrelated child.
        assert_eq!(
            unsafe { libc::kill(self.child.id() as libc::pid_t, libc::SIGTERM) },
            0
        );
    }
}

fn client(root: &Path, tls: bool) -> Client {
    let mut builder = Client::builder().no_proxy().timeout(Duration::from_secs(3));
    if tls {
        let mut pem = fs::read(root.join("client.crt")).unwrap();
        pem.push(b'\n');
        pem.extend(fs::read(root.join("client.key")).unwrap());
        builder = builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(
                Certificate::from_pem(&fs::read(root.join("ca.crt")).unwrap()).unwrap(),
            )
            .identity(Identity::from_pem(&pem).unwrap());
    }
    builder.build().unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn random_api_and_ui_ports_are_live_and_omitted_database_does_not_persist_wallets() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let client = client(root, false);
    for round in 0..2 {
        let log = root.join(format!("temporary-{round}.log"));
        let output = File::create(&log).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_quetzal"))
            .args([
                "serve",
                "--mainnet",
                "--random-port",
                "--ui-random-port",
                "--shutdown-handler",
                "--listen-address",
                "localhost",
                "--node-socket",
            ])
            .arg(root.join("absent.socket"))
            .stdin(Stdio::piped())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut socket = None;
        let mut ui_socket = None;
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "temporary service exited"
            );
            for row in fs::read_to_string(&log).unwrap().lines() {
                if let Ok(value) = serde_json::from_str::<Value>(row) {
                    let fields = &value["fields"];
                    if let Some(address) = fields["address"].as_str() {
                        match fields["message"].as_str() {
                            Some("Quetzal wallet backend starting") => {
                                socket = Some(address.parse::<SocketAddr>().unwrap())
                            }
                            Some("Quetzal personal wallet UI starting") => {
                                ui_socket = Some(address.parse::<SocketAddr>().unwrap())
                            }
                            _ => {}
                        }
                    }
                }
            }
            if socket.is_some() && ui_socket.is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "random listener startup timed out"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let socket = socket.unwrap();
        let ui_socket = ui_socket.unwrap();
        assert_ne!(socket.port(), 0);
        assert_ne!(ui_socket.port(), 0);
        let input = child.stdin.take();
        let mut service = Service {
            child,
            input,
            socket,
            ui_socket,
            url: format!("http://{socket}/v2"),
            log,
        };
        service.ready(&client).await;
        assert_eq!(
            api(&client, &service, Method::GET, "/wallets", None, 200).await,
            json!([])
        );
        if round == 0 {
            let words: Vec<_> = bip39::Mnemonic::from_entropy(&[9; 32])
                .unwrap()
                .words()
                .map(str::to_owned)
                .collect();
            api(&client,&service,Method::POST,"/wallets",Some(json!({"name":"temporary","mnemonic_sentence":words,"passphrase":"temporary-test-passphrase"})),201).await;
        }
        service.eof();
        service.exited_cleanly().await;
    }
}

async fn api(
    client: &Client,
    service: &Service,
    method: Method,
    path: &str,
    body: Option<Value>,
    status: u16,
) -> Value {
    let mut request = client.request(method, format!("{}{path}", service.url));
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body).unwrap());
    }
    let response = request.send().await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    let text = response.text().await.unwrap();
    if text.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text).unwrap()
    }
}

async fn snapshot(client: &Client, service: &Service, path: &str) -> Value {
    let wallet = api(client, service, Method::GET, path, None, 200).await;
    json!({
        "id":wallet["id"], "name":wallet["name"], "balance":wallet["balance"],
        "assets":wallet["assets"], "passphrase":wallet["passphrase"],
        "addresses":api(client, service, Method::GET, &format!("{path}/addresses"), None, 200).await,
        "transactions":api(client, service, Method::GET, &format!("{path}/transactions"), None, 200).await,
        "utxo":api(client, service, Method::GET, &format!("{path}/utxo"), None, 200).await,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_and_tls_processes_obey_stdin_eof_and_reopen_both_families() {
    for tls in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        if tls {
            test_pki::generate(root);
        }
        let client = client(root, tls);
        let mut service = Service::start(root, "open", tls, true, true);
        service.ready(&client).await;
        service
            .input
            .as_mut()
            .unwrap()
            .write_all(b"ordinary bytes\n\0not EOF")
            .unwrap();
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            service.ready(&client).await;
        }
        let mnemonic: Vec<_> = bip39::Mnemonic::from_entropy(&[7; 32])
            .unwrap()
            .to_string()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let body = json!({"name":"EOF wallet","mnemonic_sentence":mnemonic,"passphrase":"shutdown-test-passphrase"});
        let shelley = api(
            &client,
            &service,
            Method::POST,
            "/wallets",
            Some(body.clone()),
            201,
        )
        .await;
        let mut byron_body = body;
        byron_body["style"] = json!("icarus");
        let byron = api(
            &client,
            &service,
            Method::POST,
            "/byron-wallets",
            Some(byron_body),
            201,
        )
        .await;
        let paths = [
            format!("/wallets/{}", shelley["id"].as_str().unwrap()),
            format!("/byron-wallets/{}", byron["id"].as_str().unwrap()),
        ];
        let before = [
            snapshot(&client, &service, &paths[0]).await,
            snapshot(&client, &service, &paths[1]).await,
        ];
        let key = api(
            &client,
            &service,
            Method::GET,
            &format!("{}/keys", paths[0]),
            None,
            200,
        )
        .await;
        // A client that never sends ClientHello must not delay TLS shutdown.
        let _stalled = if tls {
            Some(
                tokio::net::TcpStream::connect(service.socket)
                    .await
                    .unwrap(),
            )
        } else {
            None
        };
        service.eof();
        service.exited_cleanly().await;
        drop(service);

        let mut restarted = Service::start(root, "restart", tls, true, true);
        restarted.ready(&client).await;
        for (path, before) in paths.iter().zip(before) {
            assert_eq!(snapshot(&client, &restarted, path).await, before);
        }
        assert_eq!(
            api(
                &client,
                &restarted,
                Method::GET,
                &format!("{}/keys", paths[0]),
                None,
                200
            )
            .await,
            key
        );
        restarted.eof();
        restarted.exited_cleanly().await;
        drop(restarted);

        let mut immediate = Service::start(root, "immediate", tls, true, false);
        immediate.exited_cleanly().await;
        drop(immediate);

        let mut absent = Service::start(root, "flag-absent", tls, false, false);
        absent.ready(&client).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        absent.ready(&client).await;
        absent.terminate();
        absent.exited_cleanly().await;
        drop(absent);

        // Signals must not hang runtime teardown on a still-open stdin reader.
        let mut signalled = Service::start(root, "signal-open-pipe", tls, true, true);
        signalled.ready(&client).await;
        signalled.terminate();
        signalled.exited_cleanly().await;
    }
}

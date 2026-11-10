//! The TLS listener never exposes the router before authenticating the client.
use crate::config::Config;
use anyhow::{ensure, Context, Result};
use axum::Router;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use rustls::{server::WebPkiClientVerifier, RootCertStore, ServerConfig};
use std::future::IntoFuture;
use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};
use tokio::task::JoinSet;
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

fn reader(path: &Path) -> Result<BufReader<File>> {
    Ok(BufReader::new(File::open(path).with_context(|| {
        format!("cannot open TLS file {}", path.display())
    })?))
}

pub fn load(config: &Config) -> Result<Option<Arc<ServerConfig>>> {
    config.validate_transport()?;
    let (Some(ca), Some(cert), Some(key)) = (
        &config.tls_ca_cert,
        &config.tls_server_cert,
        &config.tls_server_key,
    ) else {
        return Ok(None);
    };
    let mut roots = RootCertStore::empty();
    for certificate in rustls_pemfile::certs(&mut reader(ca)?) {
        roots.add(certificate.context("invalid TLS CA PEM")?)?;
    }
    ensure!(
        !roots.is_empty(),
        "TLS client CA file contains no certificates"
    );
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .context("cannot configure TLS client authentication")?;
    let certificates = rustls_pemfile::certs(&mut reader(cert)?)
        .collect::<std::io::Result<Vec<_>>>()
        .context("invalid TLS server certificate PEM")?;
    let key = rustls_pemfile::private_key(&mut reader(key)?)?
        .context("TLS server key file contains no private key")?;
    let mut server = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(certificates, key)
        .context("invalid TLS server certificate/key pair")?;
    server.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Some(Arc::new(server)))
}

pub async fn serve(
    listener: TcpListener,
    router: Router,
    tls: Option<Arc<ServerConfig>>,
    stop: CancellationToken,
) -> Result<()> {
    let Some(tls) = tls else {
        let serving = axum::serve(listener, router)
            .with_graceful_shutdown(stop.clone().cancelled_owned())
            .into_future();
        tokio::pin!(serving);
        tokio::select! {
            result = &mut serving => result?,
            _ = stop.cancelled() => {
                tokio::time::timeout(Duration::from_secs(10), serving)
                    .await.context("HTTP shutdown exceeded its grace period")??;
            }
        }
        return Ok(());
    };
    let acceptor = TlsAcceptor::from(tls);
    let capacity = Arc::new(Semaphore::new(1024));
    let mut connections = JoinSet::new();
    loop {
        while let Some(result) = connections.try_join_next() {
            result.context("TLS connection task panicked")?;
        }
        // Backpressure bounds both unauthenticated handshakes and open connections.
        let permit = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            permit = capacity.clone().acquire_owned() => permit?,
        };
        let (stream, _) = tokio::select! {
            biased;
            _ = stop.cancelled() => break,
            accepted = listener.accept() => accepted?,
        };
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(router.clone());
        let stop = stop.clone();
        connections.spawn(async move {
            let _permit = permit;
            let handshake = tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                result = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)) => result,
            };
            match handshake {
                Ok(Ok(stream)) => {
                    let builder = Builder::new(TokioExecutor::new());
                    let connection = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
                    tokio::pin!(connection);
                    let result = tokio::select! {
                        result = &mut connection => result,
                        _ = stop.cancelled() => {
                            connection.as_mut().graceful_shutdown();
                            connection.await
                        }
                    };
                    if let Err(error) = result {
                        tracing::debug!(%error, "TLS HTTP connection closed");
                    }
                }
                Ok(Err(error)) => tracing::debug!(%error, "TLS handshake rejected"),
                Err(_) => tracing::debug!("TLS handshake timed out"),
            }
        });
    }
    drop(listener);
    let drained = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(result) = connections.join_next().await {
            result.context("TLS connection task panicked")?;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await;
    // On a failed drain, abort and join remaining tasks before releasing state.
    connections.shutdown().await;
    drained.context("TLS shutdown exceeded its grace period")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Certificate, Client, Identity};
    use std::{error::Error, fs};

    fn config(root: &Path) -> Config {
        Config {
            tls_ca_cert: Some(root.join("ca.crt")),
            tls_server_cert: Some(root.join("server.crt")),
            tls_server_key: Some(root.join("server.key")),
            ..Config::default()
        }
    }

    fn client(
        root: &Path,
        ca: &str,
        identity: Option<&str>,
        socket: std::net::SocketAddr,
    ) -> Client {
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(3))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .tls_built_in_root_certs(false)
            .add_root_certificate(Certificate::from_pem(&fs::read(root.join(ca)).unwrap()).unwrap())
            .resolve("wrong.invalid", socket);
        if let Some(name) = identity {
            let mut pem = fs::read(root.join(format!("{name}.crt"))).unwrap();
            pem.push(b'\n');
            pem.extend(fs::read(root.join(format!("{name}.key"))).unwrap());
            builder = builder.identity(Identity::from_pem(&pem).unwrap());
        }
        builder.build().unwrap()
    }

    fn tls_error<'a>(error: &'a (dyn Error + 'static)) -> Option<&'a rustls::Error> {
        if let Some(tls) = error.downcast_ref::<rustls::Error>() {
            return Some(tls);
        }
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            if let Some(inner) = io.get_ref() {
                if let Some(tls) = tls_error(inner) {
                    return Some(tls);
                }
            }
        }
        error.source().and_then(tls_error)
    }

    async fn rejected_client(root: &Path, identity: Option<&str>, socket: std::net::SocketAddr) {
        use tokio::io::AsyncReadExt;
        let mut roots = RootCertStore::empty();
        for certificate in rustls_pemfile::certs(&mut reader(&root.join("ca.crt")).unwrap()) {
            roots.add(certificate.unwrap()).unwrap();
        }
        let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots);
        let config = if let Some(name) = identity {
            let certificates =
                rustls_pemfile::certs(&mut reader(&root.join(format!("{name}.crt"))).unwrap())
                    .collect::<std::io::Result<Vec<_>>>()
                    .unwrap();
            let key = rustls_pemfile::private_key(
                &mut reader(&root.join(format!("{name}.key"))).unwrap(),
            )
            .unwrap()
            .unwrap();
            builder.with_client_auth_cert(certificates, key).unwrap()
        } else {
            builder.with_no_client_auth()
        };
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let error = tokio::time::timeout(Duration::from_secs(3), async {
            let stream = tokio::net::TcpStream::connect(socket).await.unwrap();
            match connector
                .connect(rustls::pki_types::ServerName::from(socket.ip()), stream)
                .await
            {
                Err(error) => error,
                // TLS 1.3 may return from connect before the server checks the
                // client's Finished. Read the alert directly so HTTP dispatch
                // cancellation cannot hide the actual certificate rejection.
                Ok(mut stream) => stream
                    .read(&mut [0u8; 1024])
                    .await
                    .expect_err("unauthorized TLS client was not rejected"),
            }
        })
        .await
        .expect("a timeout does not prove certificate rejection");
        assert!(
            matches!(
                tls_error(&error),
                Some(rustls::Error::AlertReceived(
                    rustls::AlertDescription::CertificateRequired
                        | rustls::AlertDescription::BadCertificate
                        | rustls::AlertDescription::UnknownCA
                        | rustls::AlertDescription::UnsupportedCertificate
                        | rustls::AlertDescription::CertificateUnknown
                        | rustls::AlertDescription::AccessDenied
                ))
            ),
            "expected a certificate alert for {identity:?}: {error:?}"
        );
    }

    async fn positive(root: &Path, socket: std::net::SocketAddr) {
        let response = client(root, "ca.crt", Some("client"), socket)
            .get(format!("https://{socket}/v2/wallets"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response.text().await.unwrap()).unwrap(),
            serde_json::json!([])
        );
    }

    #[tokio::test]
    async fn shutdown_drains_an_admitted_request_in_http_and_tls_modes() {
        for tls in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path();
            if tls {
                crate::test_pki::generate(root);
            }
            let entered = Arc::new(tokio::sync::Notify::new());
            let finish = Arc::new(tokio::sync::Notify::new());
            let handler_entered = entered.clone();
            let handler_finish = finish.clone();
            let router = Router::new().route(
                "/inflight",
                axum::routing::get(move || {
                    let entered = handler_entered.clone();
                    let finish = handler_finish.clone();
                    async move {
                        entered.notify_one();
                        finish.notified().await;
                        "completed"
                    }
                }),
            );
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let socket = listener.local_addr().unwrap();
            let stop = CancellationToken::new();
            let server = tokio::spawn(serve(
                listener,
                router,
                if tls {
                    load(&config(root)).unwrap()
                } else {
                    None
                },
                stop.clone(),
            ));
            let client = if tls {
                client(root, "ca.crt", Some("client"), socket)
            } else {
                Client::builder().no_proxy().build().unwrap()
            };
            let request = tokio::spawn(async move {
                client
                    .get(format!(
                        "{}://{socket}/inflight",
                        if tls { "https" } else { "http" }
                    ))
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap()
            });
            tokio::time::timeout(Duration::from_secs(3), entered.notified())
                .await
                .unwrap();
            stop.cancel();
            tokio::time::sleep(Duration::from_millis(30)).await;
            assert!(
                !server.is_finished(),
                "server abandoned an admitted request"
            );
            finish.notify_one();
            assert_eq!(request.await.unwrap(), "completed");
            tokio::time::timeout(Duration::from_secs(3), server)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(tokio::net::TcpStream::connect(socket).await.is_err());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn production_transport_requires_client_auth_and_rejects_plaintext() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        crate::test_pki::generate(root);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/v2/wallets",
            axum::routing::get(|| async { axum::Json(serde_json::json!([])) }),
        );
        let stop = CancellationToken::new();
        let server = tokio::spawn(serve(
            listener,
            router,
            load(&config(root)).unwrap(),
            stop.clone(),
        ));
        for _ in 0..10 {
            for identity in [None, Some("untrusted"), Some("server")] {
                positive(root, socket).await;
                rejected_client(root, identity, socket).await;
                positive(root, socket).await;
            }
        }
        for (ca, host) in [
            ("foreign.crt", socket.to_string()),
            ("ca.crt", format!("wrong.invalid:{}", socket.port())),
        ] {
            positive(root, socket).await;
            let error = client(root, ca, Some("client"), socket)
                .get(format!("https://{host}/v2/wallets"))
                .send()
                .await
                .expect_err("invalid server certificate accepted");
            assert!(matches!(
                tls_error(&error),
                Some(rustls::Error::InvalidCertificate(_))
            ));
            positive(root, socket).await;
        }
        let error = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
            .get(format!("http://{socket}/v2/wallets"))
            .send()
            .await
            .expect_err("plaintext reached the API");
        assert!(!error.is_timeout() && !error.is_connect());
        positive(root, socket).await;
        stop.cancel();
        server.await.unwrap().unwrap();

        // Bad files/key pairs never silently select the HTTP branch.
        let mut invalid = config(root);
        invalid.tls_server_key = Some(root.join("client.key"));
        assert!(load(&invalid).is_err());
        fs::write(root.join("empty"), b"not a certificate").unwrap();
        invalid = config(root);
        invalid.tls_ca_cert = Some(root.join("empty"));
        assert!(load(&invalid).is_err());
        invalid = config(root);
        invalid.tls_server_cert = Some(root.join("missing"));
        assert!(load(&invalid).is_err());
        assert!(load(&Config::default()).unwrap().is_none());
    }
}

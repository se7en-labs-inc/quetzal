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
use std::{fs::File, io::BufReader, path::Path, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_rustls::TlsAcceptor;

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
) -> Result<()> {
    let Some(tls) = tls else {
        axum::serve(listener, router).await?;
        return Ok(());
    };
    let acceptor = TlsAcceptor::from(tls);
    let capacity = Arc::new(Semaphore::new(1024));
    loop {
        // Backpressure bounds both unauthenticated handshakes and open connections.
        let permit = capacity.clone().acquire_owned().await?;
        let (stream, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let _permit = permit;
            match tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await {
                Ok(Ok(stream)) => {
                    if let Err(error) = Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(TokioIo::new(stream), service)
                        .await
                    {
                        tracing::debug!(%error, "TLS HTTP connection closed");
                    }
                }
                Ok(Err(error)) => tracing::debug!(%error, "TLS handshake rejected"),
                Err(_) => tracing::debug!("TLS handshake timed out"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Certificate, Client, Identity};
    use std::{
        error::Error,
        fs,
        process::{Command, Stdio},
    };

    fn openssl(root: &Path, args: &[&str]) {
        let status = Command::new("openssl")
            .current_dir(root)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("Nix test environment must supply openssl");
        assert!(status.success(), "openssl fixture generation failed");
    }

    fn pki(root: &Path) {
        for ca in ["ca", "foreign"] {
            openssl(
                root,
                &[
                    "req",
                    "-x509",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-sha256",
                    "-days",
                    "2",
                    "-subj",
                    &format!("/CN={ca}"),
                    "-keyout",
                    &format!("{ca}.key"),
                    "-out",
                    &format!("{ca}.crt"),
                    "-addext",
                    "basicConstraints=critical,CA:TRUE",
                    "-addext",
                    "keyUsage=critical,keyCertSign,cRLSign",
                ],
            );
        }
        for (leaf, ca, purpose) in [
            ("server", "ca", "serverAuth"),
            ("client", "ca", "clientAuth"),
            ("untrusted", "foreign", "clientAuth"),
        ] {
            openssl(
                root,
                &[
                    "req",
                    "-new",
                    "-newkey",
                    "rsa:2048",
                    "-nodes",
                    "-sha256",
                    "-subj",
                    &format!("/CN={leaf}"),
                    "-keyout",
                    &format!("{leaf}.key"),
                    "-out",
                    &format!("{leaf}.csr"),
                ],
            );
            fs::write(root.join("extensions"), format!(
                "basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage={purpose}\nsubjectAltName=IP:127.0.0.1,DNS:localhost\n"
            )).unwrap();
            openssl(
                root,
                &[
                    "x509",
                    "-req",
                    "-in",
                    &format!("{leaf}.csr"),
                    "-CA",
                    &format!("{ca}.crt"),
                    "-CAkey",
                    &format!("{ca}.key"),
                    "-CAcreateserial",
                    "-days",
                    "2",
                    "-sha256",
                    "-extfile",
                    "extensions",
                    "-out",
                    &format!("{leaf}.crt"),
                ],
            );
        }
    }

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn production_transport_requires_client_auth_and_rejects_plaintext() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        pki(root);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/v2/wallets",
            axum::routing::get(|| async { axum::Json(serde_json::json!([])) }),
        );
        let server = tokio::spawn(serve(listener, router, load(&config(root)).unwrap()));
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
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());

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

use anyhow::{Context, Result};
use std::io::{self, Read};

fn read_to_eof(mut input: impl Read) -> io::Result<()> {
    let mut buffer = [0; 4096];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

pub async fn wait(stdin_enabled: bool) -> Result<&'static str> {
    let eof = async {
        if !stdin_enabled {
            return std::future::pending::<Result<&'static str>>().await;
        }
        let (send, receive) = tokio::sync::oneshot::channel();
        // Tokio stdin uses an uncancellable runtime blocking read. A standalone
        // thread lets signal/error shutdown finish even while the pipe is open.
        std::thread::Builder::new()
            .name("stdin-shutdown".into())
            .spawn(move || {
                let _ = send.send(read_to_eof(io::stdin().lock()));
            })
            .context("cannot start stdin shutdown reader")?;
        receive
            .await
            .context("stdin shutdown reader stopped")?
            .context("cannot read shutdown stdin")?;
        Ok("stdin EOF")
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?
            .recv()
            .await;
        Ok::<_, anyhow::Error>("SIGTERM")
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<Result<&'static str>>();
    tokio::select! {
        result = eof => result,
        result = terminate => result,
        result = tokio::signal::ctrl_c() => {
            result.context("cannot receive Ctrl-C")?;
            Ok("Ctrl-C")
        }
    }
}

use anyhow::{ensure, Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--help"] {
        println!("quetzal-faucet --config /path/to/faucet.json\nPallas-native Conway testnet ADA faucet; POST /fund, GET /health");
        return Ok(());
    }
    ensure!(
        args.len() == 2 && args[0] == "--config",
        "usage: quetzal-faucet --config FILE"
    );
    let config =
        serde_json::from_slice(&std::fs::read(&args[1]).context("cannot read faucet config")?)?;
    let faucet = quetzal_faucet::Faucet::open(config)?;
    let listener = tokio::net::TcpListener::bind(faucet.listen()).await?;
    eprintln!(
        "Quetzal development faucet listening on {}",
        listener.local_addr()?
    );
    axum::serve(listener, quetzal_faucet::router(faucet))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

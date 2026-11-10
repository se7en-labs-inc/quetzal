//! Optional personal wallet listener; Daedalus continues to use the /v2 API.
use crate::state::AppState;
use axum::{response::Html, routing::get, Router};

pub fn router(state: AppState) -> Router {
    crate::router::build_router(state).route("/", get(|| async { Html(PAGE) }))
}

const PAGE: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>Quetzal wallets</title>
<style>body{font:16px system-ui;margin:3rem auto;max-width:60rem;padding:0 1rem;color:#163b37;background:#f5faf8}article{background:white;border:1px solid #ccdcd7;border-radius:12px;padding:1rem;margin:1rem 0}code{overflow-wrap:anywhere}button{padding:.6rem 1rem}small{display:block}</style>
<h1>Quetzal wallets</h1><p id="network" role="status">Connecting to your node…</p>
<button id="refresh">Refresh</button><section id="wallets"></section>
<script>
const network=document.getElementById('network'),wallets=document.getElementById('wallets');
async function refresh(){try{
 const requests=await Promise.all(['/v2/network/information','/v2/wallets','/v2/byron-wallets'].map(path=>fetch(path)));
 if(requests.some(r=>!r.ok))throw Error('Wallet service unavailable');
 const [info,shelley,byron]=await Promise.all(requests.map(r=>r.json()));
 network.textContent=info.network_info.network_id+' · '+info.sync_progress.status+(info.sync_progress.progress?' · '+info.sync_progress.progress.quantity+'%':'');
 wallets.replaceChildren();for(const wallet of [...shelley,...byron]){
  const article=document.createElement('article'),name=document.createElement('h2'),id=document.createElement('code'),balance=document.createElement('p'),state=document.createElement('small');
  name.textContent=wallet.name;id.textContent=wallet.id;balance.textContent=wallet.balance.available.quantity+' lovelace available';state.textContent=wallet.state.status;
  article.append(name,id,balance,state);wallets.append(article);
 }if(!shelley.length&&!byron.length)wallets.textContent='No wallets restored.';
 }catch(error){network.textContent=error.message}}
document.getElementById('refresh').onclick=refresh;refresh();
</script></html>"#;

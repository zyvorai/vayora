use vayora_api::app;

#[tokio::main]
async fn main() {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("bind :8080");
    println!("vayora-api listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app()).await.expect("server");
}

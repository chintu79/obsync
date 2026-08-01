#[tokio::main]
async fn main() -> anyhow::Result<()> {
    obsync_httpd::run_server().await
}

use std::time::Duration;

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

/// Spawn the embedded Obsync server, then open the dashboard once it accepts
/// connections.
async fn boot_dashboard(app: tauri::AppHandle) -> anyhow::Result<()> {
    // Wait for the embedded HTTP server to come up before opening the window.
    for _ in 0..200 {
        if tokio::net::TcpStream::connect("127.0.0.1:42021").await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let window = WebviewWindowBuilder::new(&app, "dashboard", WebviewUrl::External(
        "http://127.0.0.1:42021".parse()?,
    ))
    .title("Obsync")
    .inner_size(1100.0, 760.0)
    .min_inner_size(700.0, 500.0)
    .build()?;

    // Re-focus when the user clicks the dock/taskbar icon.
    let _ = window;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Run the embedded httpd (dashboard + P2P sync server) until it
                // exits. Any error is logged via the window/console.
                let _ = tokio::spawn(async move {
                    if let Err(e) = obsync_httpd::run_server().await {
                        eprintln!("obsync server error: {e}");
                    }
                });
                if let Err(e) = boot_dashboard(handle).await {
                    eprintln!("failed to open dashboard: {e}");
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the dashboard window quits the app (and with it, the server).
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                window.app_handle().exit(0);
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

mod agent;
mod app;
mod overlay;
mod transcription;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = app::State::new().await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8787").await?;
    println!("OpenAIRecorder is running at http://127.0.0.1:8787");
    #[cfg(windows)]
    open_gui();
    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(windows)]
fn open_gui() {
    if std::env::var_os("RECORDER_NO_BROWSER").is_some() {
        return;
    }
    for root in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(root) {
            for suffix in [
                "Microsoft/Edge/Application/msedge.exe",
                "Google/Chrome/Application/chrome.exe",
            ] {
                let bin = std::path::PathBuf::from(&root).join(suffix);
                if bin.is_file()
                    && std::process::Command::new(bin)
                        .args(["--app=http://127.0.0.1:8787", "--window-size=1260,900"])
                        .spawn()
                        .is_ok()
                {
                    return;
                }
            }
        }
    }
    let _ = std::process::Command::new("cmd.exe")
        .args(["/c", "start", "", "http://127.0.0.1:8787"])
        .spawn();
}

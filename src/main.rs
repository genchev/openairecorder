#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod app;
#[cfg_attr(not(windows), allow(dead_code))]
mod audio;
#[cfg(windows)]
mod capture;
mod design;
mod desktop;
mod document;
mod gemini;
mod overlay;
#[cfg(windows)]
mod playback;
mod secrets;
mod segments;

fn main() -> anyhow::Result<()> {
    let result = start();
    #[cfg(windows)]
    if let Err(error) = &result {
        let text: Vec<u16> = format!("OpenAIRecorder could not start:\n{error:#}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let title: Vec<u16> = "OpenAIRecorder".encode_utf16().chain(Some(0)).collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
    result
}
fn start() -> anyhow::Result<()> {
    let web = std::env::args().any(|a| a == "--web");
    let native = !web || std::env::args().any(|a| a == "--desktop");
    #[cfg(windows)]
    let _instance = if native {
        Some(single_instance()?)
    } else {
        None
    };
    let runtime = tokio::runtime::Runtime::new()?;
    let state = runtime.block_on(app::State::new())?;
    if native {
        desktop::run(state, runtime.handle().clone())?;
    } else {
        runtime.block_on(serve_web(state))?;
    }
    Ok(())
}
async fn serve_web(state: app::State) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8787").await?;
    println!("OpenAIRecorder development web interface is running at http://127.0.0.1:8787");
    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
#[cfg(windows)]
struct Instance(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
fn single_instance() -> anyhow::Result<Instance> {
    use windows_sys::Win32::{Foundation::*, System::Threading::*};
    let name: Vec<u16> = "Local\\OpenAIRecorderDesktop"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe {
        SetLastError(0);
        CreateMutexW(std::ptr::null(), 0, name.as_ptr())
    };
    anyhow::ensure!(!handle.is_null(), "Cannot create the app's instance lock");
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        anyhow::bail!(
            "OpenAIRecorder is already running. Open its window from the Windows taskbar."
        );
    }
    Ok(Instance(handle))
}

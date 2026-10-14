use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Clone)]
pub struct Transcriber {
    pub binary: PathBuf,
    pub model: PathBuf,
    pub ffmpeg: PathBuf,
}
impl Transcriber {
    pub fn from_env() -> Self {
        Self {
            binary: std::env::var_os("WHISPER_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let local = PathBuf::from(".local-tools/whisper/Release/whisper-cli.exe");
                    if cfg!(windows) && local.is_file() {
                        local
                    } else {
                        "whisper-cli".into()
                    }
                }),
            model: std::env::var_os("WHISPER_MODEL")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let local = PathBuf::from(".local-tools/ggml-tiny.bin");
                    if local.is_file() {
                        local
                    } else {
                        "models/ggml-tiny.bin".into()
                    }
                }),
            ffmpeg: std::env::var_os("FFMPEG_BIN")
                .map(PathBuf::from)
                .unwrap_or("ffmpeg".into()),
        }
    }
    pub async fn available(&self) -> bool {
        self.model.is_file()
            && Command::new(&self.binary)
                .arg("--help")
                .output()
                .await
                .is_ok_and(|o| o.status.success())
    }
    pub async fn transcribe(&self, audio: &Path, dir: &Path) -> Result<String> {
        ensure!(
            self.model.is_file(),
            "Local speech model is missing. Follow the setup instructions in README.md."
        );
        let wav = dir.join("speech.wav");
        let conversion = Command::new(&self.ffmpeg)
            .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(audio)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
            .arg(&wav)
            .output()
            .await
            .context("Cannot start ffmpeg. Install it or set FFMPEG_BIN.")?;
        ensure!(
            conversion.status.success(),
            "Audio conversion failed. Choose a supported audio file."
        );
        let out = dir.join("transcript");
        let result = Command::new(&self.binary)
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(&wav)
            .args(["-l", "auto", "-t", "4", "-otxt", "-osrt", "-of"])
            .arg(&out)
            .output()
            .await
            .context("Cannot start whisper-cli. Set WHISPER_BIN to its installed path.")?;
        let _ = tokio::fs::remove_file(&wav).await;
        ensure!(
            result.status.success(),
            "Local transcription failed. Check the model and whisper-cli installation."
        );
        let text = tokio::fs::read_to_string(out.with_extension("txt"))
            .await
            .context("Transcription produced no text")?;
        ensure!(!text.trim().is_empty(), "No speech was detected");
        Ok(text.trim().to_owned())
    }
}

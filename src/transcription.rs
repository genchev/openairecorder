use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct SpeechSettings {
    pub language: String,
    pub model: String,
}
impl Default for SpeechSettings {
    fn default() -> Self {
        Self {
            language: "bg".into(),
            model: String::new(),
        }
    }
}
impl SpeechSettings {
    pub fn load(root: &Path) -> Self {
        std::fs::read(root.join("speech-settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}
fn bundled(path: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join(path);
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from(path)
}
fn quiet(path: &Path) -> Command {
    let command = Command::new(path);
    #[cfg(windows)]
    {
        let mut command = command;
        command.creation_flags(0x08000000);
        command
    }
    #[cfg(not(windows))]
    {
        command
    }
}
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
                    let local = bundled(".local-tools/whisper/Release/whisper-cli.exe");
                    if cfg!(windows) && local.is_file() {
                        local
                    } else {
                        "whisper-cli".into()
                    }
                }),
            model: std::env::var_os("WHISPER_MODEL")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    let local = bundled(".local-tools/ggml-tiny.bin");
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
    pub async fn available(&self, root: &Path) -> bool {
        let settings = SpeechSettings::load(root);
        let model = if settings.model.is_empty() {
            self.model.clone()
        } else {
            PathBuf::from(settings.model)
        };
        model.is_file()
            && quiet(&self.binary)
                .arg("--help")
                .output()
                .await
                .is_ok_and(|o| o.status.success())
    }
    pub async fn transcribe(&self, audio: &Path, dir: &Path) -> Result<String> {
        let settings = SpeechSettings::load(dir.parent().unwrap_or(dir));
        let model = if settings.model.is_empty() {
            self.model.clone()
        } else {
            PathBuf::from(&settings.model)
        };
        ensure!(
            model.is_file(),
            "Speech model is missing. Choose a model in Settings."
        );
        let wav = dir.join("speech.wav");
        // Native recordings are already in Whisper's required format. No ffmpeg or setup script is needed.
        let native_wav = hound::WavReader::open(audio).is_ok_and(|r| {
            let spec = r.spec();
            spec.channels == 1
                && spec.sample_rate == 16000
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int
        });
        let input = if native_wav {
            audio.to_owned()
        } else {
            let conversion = quiet(&self.ffmpeg)
                .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
                .arg(audio).args(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"]).arg(&wav)
                .output().await.context("This imported format needs ffmpeg. Native recordings work without it; alternatively import a mono 16 kHz PCM WAV.")?;
            ensure!(
                conversion.status.success(),
                "Cannot convert this imported audio format"
            );
            wav.clone()
        };
        let out = dir.join("transcript");
        let result = quiet(&self.binary)
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(&input)
            .args(["-l", &settings.language, "-t", "4", "-otxt", "-osrt", "-of"])
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn native_wav_uses_persisted_bulgarian_without_ffmpeg() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let dir = root.join("session");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let model = root.join("model.bin");
        tokio::fs::write(&model, b"fixture model").await.unwrap();
        let settings = SpeechSettings {
            language: "bg".into(),
            model: model.display().to_string(),
        };
        tokio::fs::write(
            root.join("speech-settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .await
        .unwrap();
        let binary = root.join("whisper-fixture");
        tokio::fs::write(&binary,"#!/bin/sh\nif [ \"$1\" = '--help' ]; then exit 0; fi\nprintf '%s\\n' \"$@\" > \"$0.args\"\nwhile [ $# -gt 0 ]; do\n if [ \"$1\" = '-of' ]; then shift; output=\"$1\"; fi\n shift\ndone\nprintf 'Здравейте. Това е разговор на български.' > \"$output.txt\"\n").await.unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let audio = dir.join("audio");
        let mut writer = hound::WavWriter::create(
            &audio,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample(0i16).unwrap();
        writer.finalize().unwrap();
        let transcriber = Transcriber {
            binary,
            model: root.join("absent-default-model"),
            ffmpeg: root.join("absent-ffmpeg"),
        };
        assert!(transcriber.available(&root).await);
        let result = transcriber.transcribe(&audio, &dir).await.unwrap();
        assert!(result.contains("български"));
        let args = tokio::fs::read_to_string(root.join("whisper-fixture.args"))
            .await
            .unwrap();
        assert!(args.contains("-l\nbg\n"));
        assert!(args.contains(model.to_str().unwrap()));
        assert!(!dir.join("speech.wav").exists());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}

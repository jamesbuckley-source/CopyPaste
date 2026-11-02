//! Screen recording, encoded by an ffmpeg that is already on the machine.
//!
//! The app ships no encoder: H.264 with two audio sources is thousands of lines of
//! platform API that ffmpeg has already written, and a static build would cost more
//! than the rest of the installer put together. So this module owns the *session*
//! — discovery, arguments, pausing and concatenation — and nothing else. It writes
//! an MP4 to a temporary folder and hands the path back; turning that into a video
//! clip is the commands module's job, so a recording lands in the history through
//! exactly the same store path as any other clip.
//!
//! Pausing is a segment boundary rather than an encoder feature: ffmpeg cannot
//! pause, so `pause` closes the current file with the same `q` the encoder answers
//! to a keypress, and `resume` starts the next one. `stop` joins the segments with
//! the concat demuxer, which is a copy and so costs no quality.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::capture::Rect;
use crate::error::{AppError, Result};
use crate::settings::Settings;

/// How long the encoder is given to close its file after being asked to stop.
const STOP_TIMEOUT: Duration = Duration::from_secs(8);
/// Poll interval while waiting for that.
const STOP_POLL: Duration = Duration::from_millis(40);
/// The audio device a winget install of FFmpeg is looked for under.
const WINGET_PREFIX: &str = "gyan.ffmpeg";

/// One recording request, already resolved to a rectangle on the desktop.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    pub rect: Rect,
    pub mouse: bool,
    /// DirectShow device name; empty records silent video.
    pub audio: String,
    pub fps: u32,
}

/// What the toolbar and the floating controls show.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStatus {
    /// "idle", "recording" or "paused".
    pub phase: &'static str,
    /// Milliseconds recorded so far, pauses excluded.
    pub elapsed_ms: u64,
}

impl RecordStatus {
    const IDLE: Self = Self {
        phase: "idle",
        elapsed_ms: 0,
    };
}

/// A finished recording waiting to be turned into a clip.
pub struct Finished {
    pub video: PathBuf,
    pub width: u32,
    pub height: u32,
    /// Removed once the MP4 has been moved into the history's media folder.
    folder: PathBuf,
}

impl Finished {
    /// Drops the temporary folder, after the video has been filed away.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

impl Drop for Finished {
    fn drop(&mut self) {
        // Belt and braces: a caller that never files the video must not leak it in
        // the temp folder for the rest of the session.
        if self.folder.is_dir() {
            let _ = std::fs::remove_dir_all(&self.folder);
        }
    }
}

/// The recording in progress, if any.
pub struct Recorder {
    inner: Mutex<Option<Session>>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(None),
        }
    }

    pub fn status(&self) -> RecordStatus {
        let slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            None => RecordStatus::IDLE,
            Some(session) => RecordStatus {
                phase: if session.paused_at.is_some() {
                    "paused"
                } else {
                    "recording"
                },
                elapsed_ms: session.elapsed().as_millis() as u64,
            },
        }
    }

    /// Opens the first segment. `ffmpeg` is the encoder the caller resolved, so a
    /// missing one has already been reported in its own words.
    pub fn start(&self, ffmpeg: &Path, request: RecordRequest) -> Result<()> {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(AppError::msg("a recording is already running"));
        }
        if request.rect.is_empty() {
            return Err(AppError::msg("there is nothing on screen to record"));
        }
        let folder = std::env::temp_dir().join(format!(
            "copypaste-record-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&folder)?;
        let mut session = Session {
            folder,
            ffmpeg: ffmpeg.to_path_buf(),
            segments: Vec::new(),
            child: None,
            request,
            started: Instant::now(),
            paused_for: Duration::ZERO,
            paused_at: None,
            #[cfg(windows)]
            outline: None,
        };
        if let Err(e) = session.spawn_segment() {
            let _ = std::fs::remove_dir_all(&session.folder);
            return Err(e);
        }
        // Raised after the encoder is running, so a failure to start never leaves a
        // frame blinking around nothing.
        #[cfg(windows)]
        {
            session.outline = crate::capture::RecordOutline::show(session.request.rect);
        }
        *slot = Some(session);
        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let session = slot
            .as_mut()
            .ok_or_else(|| AppError::msg("there is no recording to pause"))?;
        if session.paused_at.is_some() {
            return Ok(());
        }
        session.close_segment();
        session.paused_at = Some(Instant::now());
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        let mut slot = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let session = slot
            .as_mut()
            .ok_or_else(|| AppError::msg("there is no recording to resume"))?;
        let Some(paused_at) = session.paused_at.take() else {
            return Ok(());
        };
        session.paused_for += paused_at.elapsed();
        session.spawn_segment()
    }

    /// Closes the recording and returns the joined file, or `None` when nothing
    /// was ever recorded.
    pub fn stop(&self) -> Result<Option<Finished>> {
        // The session is taken out of the lock before the encoder is closed: that
        // wait can last seconds, and a status poll must not be stuck behind it.
        let session = self.inner.lock().unwrap_or_else(|e| e.into_inner()).take();
        let Some(mut session) = session else {
            return Ok(None);
        };
        session.close_segment();
        if session.segments.is_empty() {
            let _ = std::fs::remove_dir_all(&session.folder);
            return Ok(None);
        }
        let video = session.join_segments()?;
        let rect = even_rect(session.request.rect);
        Ok(Some(Finished {
            video,
            width: rect.width().max(0) as u32,
            height: rect.height().max(0) as u32,
            folder: session.folder,
        }))
    }
}

struct Session {
    folder: PathBuf,
    ffmpeg: PathBuf,
    segments: Vec<PathBuf>,
    child: Option<Child>,
    request: RecordRequest,
    started: Instant,
    paused_for: Duration,
    paused_at: Option<Instant>,
    /// The blinking frame around the area being recorded, dropped with the session.
    #[cfg(windows)]
    outline: Option<crate::capture::RecordOutline>,
}

impl Session {
    fn elapsed(&self) -> Duration {
        let running = self
            .paused_at
            .map(|paused_at| paused_at.elapsed())
            .unwrap_or_default();
        self.started
            .elapsed()
            .saturating_sub(self.paused_for)
            .saturating_sub(running)
    }

    fn spawn_segment(&mut self) -> Result<()> {
        let out = self.folder.join(format!("{:03}.mp4", self.segments.len()));
        let mut command = Command::new(&self.ffmpeg);
        command
            .args(segment_args(&self.request, &out))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        hide_console(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| AppError::msg(format!("could not start FFmpeg: {e}")))?;
        // Drained on its own thread: a full stderr pipe would stall the encoder.
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(|line| line.ok()) {
                    log::warn!("ffmpeg: {line}");
                }
            });
        }
        self.segments.push(out);
        self.child = Some(child);
        Ok(())
    }

    /// Asks the encoder to close its file, then makes sure it did.
    fn close_segment(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }
        // `q` is answered once the file's index is written; killing sooner would
        // leave an unplayable segment.
        let deadline = Instant::now() + STOP_TIMEOUT;
        while Instant::now() < deadline {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(STOP_POLL),
                Err(e) => {
                    log::warn!("could not wait for FFmpeg: {e}");
                    break;
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Joins the segments into one file, or renames the single one.
    fn join_segments(&mut self) -> Result<PathBuf> {
        let out = self.folder.join("recording.mp4");
        if self.segments.len() == 1 {
            std::fs::rename(&self.segments[0], &out)?;
            return Ok(out);
        }
        let list = self.folder.join("segments.txt");
        let mut body = String::new();
        for segment in &self.segments {
            // The concat demuxer wants forward slashes and an escaped quote.
            let path = segment.to_string_lossy().replace('\\', "/").replace('\'', "'\\''");
            body.push_str(&format!("file '{path}'\n"));
        }
        std::fs::write(&list, body)?;

        let mut command = Command::new(&self.ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&list)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        hide_console(&mut command);
        let output = command
            .output()
            .map_err(|e| AppError::msg(format!("could not join the recording: {e}")))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::msg(format!(
                "could not join the recording: {}",
                first_line(&detail)
            )));
        }
        Ok(out)
    }
}

/// The arguments for one segment: gdigrab for the pixels, dshow for the device.
///
/// Kept separate from the process so the shape ffmpeg is handed can be tested.
pub fn segment_args(request: &RecordRequest, out: &Path) -> Vec<String> {
    let rect = even_rect(request.rect);
    let fps = request.fps.clamp(5, 60);
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-f".into(),
        "gdigrab".into(),
        "-framerate".into(),
        fps.to_string(),
        "-draw_mouse".into(),
        if request.mouse { "1" } else { "0" }.into(),
        "-offset_x".into(),
        rect.left.to_string(),
        "-offset_y".into(),
        rect.top.to_string(),
        "-video_size".into(),
        format!("{}x{}", rect.width(), rect.height()),
        "-i".into(),
        "desktop".into(),
    ];
    let audio = request.audio.trim();
    if !audio.is_empty() {
        args.extend([
            "-f".into(),
            "dshow".into(),
            "-i".into(),
            format!("audio={audio}"),
        ]);
    }
    args.extend([
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-crf".into(),
        "23".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        // A keyframe every two seconds keeps the pause segments cheap to join and
        // the result seekable.
        "-g".into(),
        (fps * 2).to_string(),
    ]);
    if !audio.is_empty() {
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "128k".into(),
            "-ac".into(),
            "2".into(),
        ]);
    }
    args.push(out.to_string_lossy().into_owned());
    args
}

/// libx264 refuses an odd frame size, so the rectangle loses its last column or
/// row when it has to.
pub fn even_rect(rect: Rect) -> Rect {
    let width = rect.width() - rect.width() % 2;
    let height = rect.height() - rect.height() % 2;
    Rect::new(rect.left, rect.top, rect.left + width.max(0), rect.top + height.max(0))
}

/// Writes one frame of `video` to `out`, for the still a video clip shows before it
/// plays.
pub fn poster(ffmpeg: &Path, video: &Path, out: &Path) -> Result<()> {
    // A tenth of a second in skips the fade-in most recordings start on, but a
    // recording shorter than that would produce nothing, so the retry starts at 0.
    for seek in ["0.1", "0"] {
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", seek])
            .arg("-i")
            .arg(video)
            .args(["-frames:v", "1"])
            .arg(out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        hide_console(&mut command);
        let output = command
            .output()
            .map_err(|e| AppError::msg(format!("could not read a frame from the recording: {e}")))?;
        if output.status.success() && out.is_file() {
            return Ok(());
        }
    }
    Err(AppError::msg(
        "could not read a frame from the recording, so it has no preview",
    ))
}

/// Moves a finished recording into the media folder.
///
/// `rename` is the right call when both ends are on one volume, and fails with a
/// cross-device error the moment they are not — which is the normal case for a
/// cloud-backed or portable history folder on another drive. Then it is a copy and
/// a delete, so a recording is never lost to the drive letter it landed on.
pub fn file_into_place(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

/// The ffmpeg this app will use, if there is one.
///
/// A configured path wins; otherwise `ffmpeg` is taken from `PATH`, and finally
/// from the folder a winget install leaves it in, which is not on `PATH` until the
/// next sign-in.
pub fn ffmpeg_path(settings: &Settings) -> Option<PathBuf> {
    let configured = settings.record_ffmpeg_path.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        return path.is_file().then_some(path);
    }
    probe_ffmpeg().or_else(winget_ffmpeg)
}

fn probe_ffmpeg() -> Option<PathBuf> {
    let mut command = Command::new("ffmpeg");
    command
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
    command.status().ok()?.success().then(|| PathBuf::from("ffmpeg"))
}

fn winget_ffmpeg() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")?;
    let packages = PathBuf::from(base)
        .join("Microsoft")
        .join("WinGet")
        .join("Packages");
    for package in std::fs::read_dir(packages).ok()?.flatten() {
        let name = package.file_name().to_string_lossy().to_ascii_lowercase();
        if !name.starts_with(WINGET_PREFIX) {
            continue;
        }
        for build in std::fs::read_dir(package.path()).ok()?.flatten() {
            let candidate = build.path().join("bin").join("ffmpeg.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Installs FFmpeg with winget, for the machines that have neither a copy on
/// `PATH` nor a configured one.
pub fn install_ffmpeg() -> Result<String> {
    let mut command = Command::new("winget");
    command
        .args([
            "install",
            "--id",
            "Gyan.FFmpeg",
            "-e",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let output = command.output().map_err(|e| {
        AppError::msg(format!(
            "winget is not available on this machine, so FFmpeg has to be installed by hand: {e}"
        ))
    })?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !output.status.success() {
        return Err(AppError::msg(format!(
            "winget could not install FFmpeg: {}",
            first_line(&text)
        )));
    }
    Ok(first_line(&text))
}

/// The audio devices a DirectShow capture can be started with.
pub fn audio_devices(ffmpeg: &Path) -> Vec<String> {
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_console(&mut command);
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    parse_devices(&String::from_utf8_lossy(&output.stderr))
}

/// Pulls the audio device names out of `-list_devices` output.
///
/// Two shapes have to be understood. FFmpeg up to 8 grouped the list under
/// "DirectShow video devices" and "DirectShow audio devices" headers; FFmpeg 9
/// prints one flat list and tags each line `(audio)`, `(video)` or `(none)`
/// instead. The tag wins when it is there, and the section decides when it is not,
/// so one parser serves both.
pub fn parse_devices(text: &str) -> Vec<String> {
    let mut devices: Vec<String> = Vec::new();
    let mut in_audio_section = false;
    for line in text.lines() {
        if line.contains("DirectShow video devices") {
            in_audio_section = false;
            continue;
        }
        if line.contains("DirectShow audio devices") {
            in_audio_section = true;
            continue;
        }
        if line.contains("Alternative name") {
            continue;
        }
        let Some(name) = quoted(line) else {
            continue;
        };
        let audio = if line.contains("(audio)") {
            true
        } else if line.contains("(video)") || line.contains("(none)") {
            false
        } else {
            in_audio_section
        };
        if audio && !devices.contains(&name) {
            devices.push(name);
        }
    }
    devices
}

fn quoted(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

/// The first line of a command's output, for an error message that fits in a toast.
fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_string()
}

/// Keeps a console window from flashing up for every encoder start.
fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(rect: Rect, audio: &str, mouse: bool) -> RecordRequest {
        RecordRequest {
            rect,
            mouse,
            audio: audio.into(),
            fps: 30,
        }
    }

    #[test]
    fn a_silent_region_asks_only_for_the_desktop() {
        let args = segment_args(
            &request(Rect::new(100, 200, 900, 700), "", true),
            Path::new("out.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-f gdigrab"), "{joined}");
        assert!(joined.contains("-offset_x 100 -offset_y 200"), "{joined}");
        assert!(joined.contains("-video_size 800x500"), "{joined}");
        assert!(joined.contains("-draw_mouse 1"), "{joined}");
        assert!(!joined.contains("dshow"), "no device was chosen");
        assert!(!joined.contains("-c:a"), "silent video needs no audio codec");
        assert_eq!(args.last().unwrap(), "out.mp4");
    }

    #[test]
    fn a_negative_offset_is_passed_through_for_a_monitor_left_of_the_primary() {
        let args = segment_args(
            &request(Rect::new(-1920, 0, 0, 1080), "", false),
            Path::new("out.mp4"),
        );
        let joined = args.join(" ");
        assert!(joined.contains("-offset_x -1920"), "{joined}");
        assert!(joined.contains("-draw_mouse 0"), "{joined}");
    }

    #[test]
    fn a_chosen_device_becomes_a_dshow_input() {
        let args = segment_args(
            &request(
                Rect::new(0, 0, 640, 480),
                "Microphone (Realtek(R) Audio)",
                true,
            ),
            Path::new("out.mp4"),
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("-f dshow -i audio=Microphone (Realtek(R) Audio)"),
            "{joined}"
        );
        assert!(joined.contains("-c:a aac"), "{joined}");
    }

    #[test]
    fn an_odd_rectangle_loses_its_last_column_not_the_whole_recording() {
        let rect = even_rect(Rect::new(10, 10, 645, 481));
        assert_eq!((rect.width(), rect.height()), (634, 470));
        assert_eq!((rect.left, rect.top), (10, 10));

        let args = segment_args(
            &request(Rect::new(10, 10, 645, 481), "", true),
            Path::new("out.mp4"),
        );
        assert!(args.join(" ").contains("-video_size 634x470"));
    }

    #[test]
    fn the_device_list_splits_audio_from_video() {
        let text = r#"
[dshow @ 000] "Integrated Camera" (video)
[dshow @ 000]   Alternative name "@device_pnp_\\?\usb#vid"
[dshow @ 000] DirectShow audio devices
[dshow @ 000]  "Microphone (Realtek(R) Audio)"
[dshow @ 000]   Alternative name "@device_cm_{33D9A762}\wave_{1F2E}"
[dshow @ 000]  "Stereo Mix (Realtek(R) Audio)"
"#;
        assert_eq!(
            parse_devices(text),
            vec![
                "Microphone (Realtek(R) Audio)".to_string(),
                "Stereo Mix (Realtek(R) Audio)".to_string()
            ]
        );
    }

    #[test]
    fn ffmpeg_nine_tags_each_device_instead_of_grouping_them() {
        // Verbatim from FFmpeg 9.0.2, which prints no section headers at all: the
        // audio half of the list used to vanish because of it.
        let text = r#"ffmpeg version 9.0.2-full_build
[in#0 @ 000002dd] "Snap Camera" (video)
[in#0 @ 000002dd]   Alternative name "@device_pnp_\\?\root#camera#0000"
[in#0 @ 000002dd] "OBS-Camera" (none)
[in#0 @ 000002dd]   Alternative name "@device_sw_{860BB310-5D01-11D0-BD3B-00A0C911CE86}\{27B05C2D}"
[in#0 @ 000002dd] "Microphone (Yeti Stereo Microphone)" (audio)
[in#0 @ 000002dd]   Alternative name "@device_cm_{33D9A762-90C8-11D0-BD43-00A0C911CE86}\wave_{83E01E9F}"
[in#0 @ 000002dd] "OBS-Audio" (none)
[in#0 @ 000002dd] "Stereo Mix (Realtek(R) Audio)" (audio)
[in#0 @ 000002dd] "Microphone (Yeti Stereo Microphone)" (audio)
Error opening input file dummy.
"#;
        assert_eq!(
            parse_devices(text),
            vec![
                "Microphone (Yeti Stereo Microphone)".to_string(),
                "Stereo Mix (Realtek(R) Audio)".to_string()
            ],
            "audio in the flat list, listed once, and the video devices left out"
        );
    }

    #[test]
    fn a_list_with_no_audio_section_offers_nothing() {
        assert!(parse_devices("[dshow @ 000] DirectShow video devices").is_empty());
    }

    #[test]
    fn a_finished_recording_is_moved_off_the_temp_folder() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("recording.mp4");
        std::fs::write(&source, b"pretend this is a movie").unwrap();
        let target = folder.path().join("7.mp4");

        file_into_place(&source, &target).unwrap();

        assert!(target.is_file(), "the clip's media has to exist");
        assert!(!source.exists(), "the temporary copy must not be left behind");
        assert_eq!(std::fs::read(&target).unwrap(), b"pretend this is a movie");
    }

    #[test]
    fn the_frame_rate_is_clamped_to_what_an_encoder_can_keep_up_with() {
        let mut fast = request(Rect::new(0, 0, 100, 100), "", true);
        fast.fps = 1_000;
        let args = segment_args(&fast, Path::new("out.mp4"));
        assert!(args.join(" ").contains("-framerate 60"));
    }

    /// Checks the whole discovery path against this machine's real FFmpeg: the
    /// encoder is found and its audio devices are parsed out of whatever list
    /// format the installed version prints. Run with
    /// `cargo test the_machines_audio_devices_are_found -- --ignored`.
    #[test]
    #[ignore = "needs FFmpeg and a capture device on this machine"]
    fn the_machines_audio_devices_are_found() {
        let ffmpeg = ffmpeg_path(&Settings::default()).expect("an encoder is needed");
        let devices = audio_devices(&ffmpeg);
        assert!(
            !devices.is_empty(),
            "no DirectShow audio devices were parsed from {}",
            ffmpeg.display()
        );
        println!("{}: {devices:?}", ffmpeg.display());
    }

    /// End-to-end smoke test: the argument list above, a real encoder, a real
    /// desktop. Ignored by default because it needs both, and because it takes a
    /// couple of seconds; run it with
    /// `cargo test recording_a_region_produces_a_playable_file -- --ignored`.    #[test]
    #[ignore = "needs FFmpeg on PATH and a desktop to record"]
    fn recording_a_region_produces_a_playable_file() {
        let ffmpeg = ffmpeg_path(&Settings::default()).expect("ffmpeg is needed for this test");
        let folder = std::env::temp_dir().join("copypaste-record-test");
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let out = folder.join("smoke.mp4");

        let mut command = Command::new(&ffmpeg);
        command
            .args(segment_args(
                &request(Rect::new(0, 0, 320, 240), "", false),
                &out,
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut command);
        let mut child = command.spawn().expect("ffmpeg should start");
        std::thread::sleep(Duration::from_secs(2));
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"q\n")
            .expect("the encoder should accept the stop key");
        child.wait().expect("ffmpeg should finish");

        let size = std::fs::metadata(&out).expect("the recording should exist").len();
        assert!(size > 1_000, "a two second recording was only {size} bytes");
        let _ = std::fs::remove_dir_all(&folder);
    }
}

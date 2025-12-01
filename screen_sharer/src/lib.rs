use livekit::track::LocalTrack;
use livekit::webrtc::desktop_capturer::{
    CaptureError, DesktopCaptureSourceType, DesktopCapturer, DesktopCapturerOptions,
    DesktopFrame,
};
use livekit::webrtc::native::yuv_helper;
use livekit::webrtc::prelude::VideoBuffer;
use livekit::webrtc::prelude::{NV12Buffer, VideoFrame, VideoResolution, VideoRotation};
use livekit::webrtc::video_source::native::NativeVideoSource;
use livekit::RoomEvent;
use std::cmp::max;
use std::fs::File;
use std::io::Write;
use std::sync::{mpsc, Arc, Mutex};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub struct ScreenSharer {
    capturer: Arc<Mutex<DesktopCapturer>>,
    watermark_count: Arc<Mutex<u32>>,
    buffer_source: NativeVideoSource,
    tx: Option<mpsc::Sender<ScreenshareMessage>>,
    source_index: u32,
}

fn get_source_dims(source_index: u32) -> (u32, u32) {
    let width = Arc::new(Mutex::new(0));
    let height = Arc::new(Mutex::new(0));

    let width_clone = width.clone();
    let height_clone = height.clone();
    let callback = move |result: Result<DesktopFrame, CaptureError>| {
        match result {
            Ok(frame) => {
                let (width, height) = (frame.width(), frame.height());
                *width_clone.lock().unwrap() = width as u32;
                *height_clone.lock().unwrap() = height as u32;
            }
            Err(error) => {
                log::warn!("Capture error: {:?}", error);
            }
        }
    };
    let mut options = DesktopCapturerOptions::new(DesktopCaptureSourceType::Screen);
    #[cfg(target_os = "macos")]
    {
        options.set_sck_system_picker(false);
    }
    let mut capturer = DesktopCapturer::new(options).unwrap();
    let source = capturer
        .get_source_list()
        .get(source_index as usize)
        .cloned();
    capturer.start_capture(source, callback);
    let mut count = 0;
    while count < 10 {
        capturer.capture_frame();

        if *width.lock().unwrap() > 0 && *height.lock().unwrap() > 0 {
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        count += 1;
    }

    (
        *width.lock().unwrap(),
        *height.lock().unwrap(),
    )
}

pub fn aspect_fit(width: u32, height: u32, target_width: u32, target_height: u32) -> (u32, u32) {
    let size = max(target_width, target_height);
    if width >= height {
        let aspect_ratio = height as f32 / width as f32;
        (size, ((size as f32) * aspect_ratio) as u32)
    } else {
        let aspect_ratio = width as f32 / height as f32;
        (((size as f32) / aspect_ratio) as u32, size)
    }
}

impl ScreenSharer {
    pub fn new(width: u32, height: u32, source_index: u32) -> Result<Self, ()> {
        let (screen_width, screen_height) = get_source_dims(source_index);
        log::info!(
            "Screen source dimensions: {}x{}",
            screen_width,
            screen_height
        );

        let (width, height) = aspect_fit(screen_width, screen_height, width, height);

        let buffer_source = NativeVideoSource::new(VideoResolution { width, height });
        let watermark_count = Arc::new(Mutex::new(0));
        let watermark_count_clone = watermark_count.clone();

        let buffer_source_clone = buffer_source.clone();
        let video_frame = Mutex::new(VideoFrame {
            rotation: VideoRotation::VideoRotation0,
            buffer: NV12Buffer::new(width, height),
            timestamp_us: 0,
        });
        let tmp_buffer = Mutex::new(NV12Buffer::new(screen_width, screen_height));
        let callback = move |result: Result<DesktopFrame, CaptureError>| {
            let frame = match result {
                Ok(frame) => frame,
                Err(error) => {
                    log::warn!("Capture error: {:?}", error);
                    return;
                }
            };

            let height = frame.height();
            let width = frame.width();
            let stride = frame.stride();
            let data = frame.data();

            let mut buffer = tmp_buffer.lock().unwrap();
            let (s_y, s_uv) = buffer.strides();
            let (y, uv) = buffer.data_mut();
            yuv_helper::argb_to_nv12(data, stride, y, s_y, uv, s_uv, width, height);

            // Scale framebuffer to stream resolution
            let mut stream_buffer = video_frame.lock().unwrap();
            let stream_width = stream_buffer.buffer.width();
            let stream_height = stream_buffer.buffer.height();

            let mut scaled_buffer = buffer.scale(stream_width as i32, stream_height as i32);

            // Copy scaled buffer to stream buffer
            let (data_y, data_uv) = scaled_buffer.data_mut();
            let (s_y, _) = stream_buffer.buffer.strides();
            let (dst_y, dst_uv) = stream_buffer.buffer.data_mut();
            dst_y.copy_from_slice(data_y);
            dst_uv.copy_from_slice(data_uv);

            {
                let mut watermark_count = watermark_count_clone.lock().unwrap();
                if *watermark_count > 0 {
                    *watermark_count -= 1;
                    unsafe {
                        let dst = dst_y.as_mut_ptr();
                        std::ptr::write_bytes(dst, 0xa, (50 * s_y) as usize);
                    }
                }
            }

            buffer_source_clone.capture_frame(&stream_buffer);
        };
        let mut options = DesktopCapturerOptions::new(DesktopCaptureSourceType::Screen);
        #[cfg(target_os = "macos")]
        {
            options.set_sck_system_picker(false);
        }
        let capturer = DesktopCapturer::new(options);
        if capturer.is_none() {
            return Err(());
        }

        let mut capturer = capturer.unwrap();
        let source = capturer
            .get_source_list()
            .get(source_index as usize)
            .cloned();
        capturer.start_capture(source, callback);

        Ok(ScreenSharer {
            capturer: Arc::new(Mutex::new(capturer)),
            watermark_count: watermark_count,
            buffer_source,
            tx: None,
            source_index,
        })
    }

    pub fn buffer_source(&self) -> NativeVideoSource {
        self.buffer_source.clone()
    }

    pub fn start_capture(&mut self, room: livekit::Room) {
        let (tx, rx) = mpsc::channel();
        self.tx = Some(tx);

        let capturer = self.capturer.clone();
        std::thread::spawn(move || {
            run_capture_frame(rx, capturer, room);
        });
    }

    pub fn stop_capture(&mut self, encoder: &str, resolution: &str, bitrate: u64, name: &str) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(ScreenshareMessage::StopCapture {
                encoder: encoder.to_string(),
                resolution: resolution.to_string(),
                bitrate,
                name: name.to_string(),
            });
        }
    }

    pub fn watermark_count(&self) -> Arc<Mutex<u32>> {
        self.watermark_count.clone()
    }
}

enum ScreenshareMessage {
    StopCapture {
        encoder: String,
        resolution: String,
        bitrate: u64,
        name: String,
    },
}

fn run_capture_frame(
    rx: mpsc::Receiver<ScreenshareMessage>,
    capturer: Arc<Mutex<DesktopCapturer>>,
    room: livekit::Room,
) {
    let mut frames = 0;
    let pid = std::process::id() as usize;
    let mut system = System::new_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    // Refresh CPU usage to get actual value.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );
    let mut stats = Vec::<Stats>::new();
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(16)) {
            Ok(ScreenshareMessage::StopCapture {
                encoder,
                resolution,
                bitrate,
                name,
            }) => {
                // Write CPU usage data to CSV file
                let filename = format!("{}_{}_{}_{}.csv", encoder, resolution, bitrate, name);
                if let Ok(mut file) = File::create(&filename) {
                    let _ = writeln!(file, "frame,cpu_usage,bytes_sent");
                    for (i, stat) in stats.iter().enumerate() {
                        let _ = writeln!(file, "{},{:.2},{:.2}", i, stat.cpu_usage, stat.bytes_sent);
                    }
                    log::info!("encoder stats data saved to {}", filename);
                } else {
                    log::error!("Failed to create encoder stats file: {}", filename);
                }
                break;
            }
            Err(e) => match e {
                mpsc::RecvTimeoutError::Timeout => {
                    let mut capturer = capturer.lock().unwrap();
                    capturer.capture_frame();
                    frames += 1;
                    if frames % 150 == 0 {
                        system.refresh_processes_specifics(
                            ProcessesToUpdate::All,
                            true,
                            ProcessRefreshKind::nothing().with_cpu(),
                        );
                        let mut cpu = 0.;
                        if let Some(process) = system.process(Pid::from(pid)) {
                            cpu = process.cpu_usage();
                        } else {
                            log::warn!("Process with PID {} not found", pid);
                        }

                        stats.push(pollster::block_on(get_rtc_stats(&room, cpu)));
                    }
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    log::error!("run_capture_frame: Disconnected");
                    break;
                }
            },
        }
    }
}

struct Stats {
    bytes_sent: u64,
    cpu_usage: f32,
}

async fn get_rtc_stats(room: &livekit::Room, cpu_usage: f32) -> Stats {
    let mut ret_stats = Stats {
        bytes_sent: 0,
        cpu_usage,
    };
    let local_participant = room.local_participant();
    for (_, publication) in local_participant.track_publications() {
        let track = publication.track();
        if track.is_none() {
            continue;
        }
        let track = track.unwrap();
        if let LocalTrack::Video(track) = track {
            let stats = track.get_stats().await.unwrap();
            for stat in stats {
                match stat {
                    livekit::webrtc::stats::RtcStats::CandidatePair(stats) => {
                        ret_stats.bytes_sent = stats.candidate_pair.bytes_sent;
                    }
                    livekit::webrtc::stats::RtcStats::MediaSource(stats) => {
                        let frames_sent = stats.video.frames;
                        log::info!("Media Source Frames Sent: {}", frames_sent);
                    }
                    livekit::webrtc::stats::RtcStats::OutboundRtp(stats) => {
                        let frames_sent = stats.outbound.frames_sent;
                        let quality_limitation = stats.outbound.quality_limitation_reason;
                        let quality_limitation_value = stats.outbound.quality_limitation_durations;
                        let frame_width = stats.outbound.frame_width;
                        let frame_height = stats.outbound.frame_height;
                        let target_bitrate = stats.outbound.target_bitrate;
                        let fps = stats.outbound.frames_per_second;
                        let total_encode_time = stats.outbound.total_encode_time;
                        log::info!(
                            "Outbound RTP Frames Sent: {}, Quality Limitation: {:?}, Quality Limitation Value: {:?}, Frame Size: {}x{}, Target Bitrate: {}, FPS: {}, Total Encode Time: {}",
                            frames_sent,
                            quality_limitation,
                            quality_limitation_value,
                            frame_width,
                            frame_height,
                            target_bitrate,
                            fps,
                            total_encode_time,
                        );
                    }
                    _ => {}
                }
            }
        }
    }

    log::info!("Stats: Total Bytes Sent: {}, CPU Usage: {:.2}%", ret_stats.bytes_sent, ret_stats.cpu_usage);
    ret_stats
}

pub fn handle_room_events(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    watermark_count: Arc<Mutex<u32>>,
) {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                RoomEvent::DataReceived { payload, .. } => {
                    let received_string = String::from_utf8_lossy(&payload);
                    if received_string == "watermark" {
                        log::info!("Watermark received, setting count to 10");
                        let mut count = watermark_count.lock().unwrap();
                        *count = 15;
                    }
                }
                _ => {}
            }
        }
    });
}

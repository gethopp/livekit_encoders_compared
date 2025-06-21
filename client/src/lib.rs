use futures::StreamExt;
use livekit::{
    prelude::*,
    webrtc::{prelude::RtcVideoTrack, video_stream::native::NativeVideoStream},
};
use std::fs::File;
use std::io::{self, Write};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
#[derive(Debug, Clone, Copy)]
struct LatencyEntry {
    id: u64,
    timestamp: u128,
    receive_timestamp: u128,
    rtc_stats: Option<LatencyStats>,
    cpu_usage: f32,
}

impl std::fmt::Display for LatencyEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(stats) = &self.rtc_stats {
            write!(
                f,
                "{} latency: {} stats: {}, cpu_usage: {}",
                self.id,
                self.receive_timestamp - self.timestamp,
                stats,
                self.cpu_usage
            )
        } else {
            write!(
                f,
                "{} latency: {} stats: no stats available",
                self.id,
                self.receive_timestamp - self.timestamp
            )
        }
    }
}

/*
 * jitter_buffer_minimum_delay, freeze_count and total_bytes we
 * use only the last value.
 */
#[derive(Debug, Clone, Copy)]
struct LatencyStats {
    processing_delay: f64,
    jitter_buffer_delay: f64,
    jitter_buffer_target_delay: f64,
    jitter_buffer_minimum_delay: f64,
    frames_per_second: f64,
    total_frames: f64,
    freeze_count: f64,
    total_bytes: f64,
    dropped_frames: f64,
}

impl std::fmt::Display for LatencyStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "processing_delay: {}, jitter_buffer_delay: {}, jitter_buffer_target_delay: {}, jitter_buffer_minimum_delay: {}, frames_per_second: {:.2}, total_frames: {}, freeze_count: {}, total_bytes: {}, dropped_frames: {}",
            self.processing_delay,
            self.jitter_buffer_delay,
            self.jitter_buffer_target_delay,
            self.jitter_buffer_minimum_delay,
            self.frames_per_second,
            self.total_frames,
            self.freeze_count,
            self.total_bytes,
            self.dropped_frames
        )
    }
}

async fn get_rtc_stats(room: &Room) -> LatencyStats {
    let mut latency_stats = LatencyStats {
        processing_delay: 0.,
        jitter_buffer_delay: 0.,
        jitter_buffer_target_delay: 0.,
        jitter_buffer_minimum_delay: 0.,
        frames_per_second: 0.,
        freeze_count: 0.,
        total_bytes: 0.,
        dropped_frames: 0.,
        total_frames: 0.,
    };
    for (_, remote_participant) in room.remote_participants() {
        for (_, publication) in remote_participant.track_publications() {
            let track = publication.track();
            if track.is_none() {
                continue;
            }
            let track = track.unwrap();
            if let RemoteTrack::Video(track) = track {
                let stats = track.get_stats().await.unwrap();
                for stat in stats {
                    match stat {
                        livekit::webrtc::stats::RtcStats::InboundRtp(stats) => {
                            let processing_delay = (stats.inbound.total_processing_delay as f64)
                                / (stats.inbound.frames_decoded as f64)
                                * 1000.;
                            let jitter_buffer_delay = (stats.inbound.jitter_buffer_delay as f64)
                                / (stats.inbound.jitter_buffer_emitted_count as f64)
                                * 1000.;
                            let jitter_buffer_target_delay =
                                (stats.inbound.jitter_buffer_target_delay as f64)
                                    / (stats.inbound.jitter_buffer_emitted_count as f64)
                                    * 1000.;
                            let jitter_buffer_minimum_delay =
                                (stats.inbound.jitter_buffer_minimum_delay as f64)
                                    / (stats.inbound.jitter_buffer_emitted_count as f64)
                                    * 1000.;
                            let total_bytes = stats.inbound.bytes_received as f64;
                            latency_stats = LatencyStats {
                                processing_delay,
                                jitter_buffer_delay,
                                jitter_buffer_target_delay,
                                jitter_buffer_minimum_delay,
                                frames_per_second: stats.inbound.frames_per_second,
                                freeze_count: stats.inbound.freeze_count as f64,
                                total_bytes,
                                dropped_frames: stats.inbound.frames_dropped as f64,
                                total_frames: stats.inbound.frames_received as f64,
                            };
                        },
                        _ => {}
                    }
                }
            }
        }
    }
    latency_stats
}

async fn measure_latency(room: Room, track: RtcVideoTrack) -> Vec<LatencyEntry> {
    let pid = std::process::id() as usize;
    let mut system = System::new_all();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    // Refresh CPU usage to get actual value.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );

    /* Vector for storing the measurements. */
    let mut latency_results: Vec<LatencyEntry> = vec![];
    /* Total frame counter. */
    let mut frames = 0;
    /* Next frame to send tick. */
    let mut next_frame_request = 0;
    /* Send ticks every frames_offset frames. */
    let frames_offset = 150;

    /* FPS calculation variables */
    let mut start_time = std::time::SystemTime::now();
    let mut last_frame_for_fps = 0;

    let mut video_sink = NativeVideoStream::new(track);
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(10000), video_sink.next()).await
    {
        let receive_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        /*
         * Access the buffer and read the first 200
         * Y samples.
         */
        let buffer = frame.buffer.to_i420();
        let (data_y, _, _) = buffer.data();

        let mut watermark_count = 0;
        for i in 0..200 {
            if data_y[i] == 0xa {
                watermark_count += 1;
            }
        }

        /* Limit for accepting the watermark. */
        let min_watermark_count = 100;
        /* Delay sampling by 500 frames. */
        let start_sampling_frame = 500;
        if watermark_count >= min_watermark_count && frames > start_sampling_frame {
            if let Some(entry) = latency_results.last_mut() {
                /* If the entry has a receive timestamp don't overwrite it. */
                if entry.receive_timestamp == 0 {
                    entry.receive_timestamp = receive_timestamp;

                    /* Get rtc stats. */
                    let rtc_stats = get_rtc_stats(&room).await;
                    entry.rtc_stats = Some(rtc_stats);

                    system.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::nothing().with_cpu(),
                    );
                    if let Some(process) = system.process(Pid::from(pid)) {
                        entry.cpu_usage = process.cpu_usage();
                    } else {
                        log::warn!("Process with PID {} not found", pid);
                    }

                    /* Calculate local FPS every second */
                    let elapsed_time_since_start = start_time.elapsed().unwrap().as_secs();
                    let frames_per_second = (frames - last_frame_for_fps) as f64 / elapsed_time_since_start as f64;
                    entry.rtc_stats.as_mut().unwrap().frames_per_second =
                        frames_per_second;

                    log::info!("{}", entry);
                    start_time = std::time::SystemTime::now();
                    last_frame_for_fps = frames;
                }
            }
        }

        /* Send tick and create next measurement entry. */
        if frames == next_frame_request {
            next_frame_request += frames_offset;
            /* Trigger next measurement frame. */
            room.local_participant()
                .publish_data(DataPacket {
                    payload: "watermark".to_owned().into_bytes(),
                    reliable: true,
                    ..Default::default()
                })
                .await
                .unwrap();

            /* Create new measurement entry. */
            latency_results.push(LatencyEntry {
                id: next_frame_request / frames_offset,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
                receive_timestamp: 0,
                rtc_stats: None,
                cpu_usage: 0.,
            });
        }
        frames += 1;
    }

    latency_results
}

pub async fn end_to_end_latency(
    room: Room,
    track: RemoteVideoTrack,
    output_file: &str,
) -> io::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let latency = measure_latency(room, track.rtc_track()).await;
    let end = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let duration = end - now;
    write_latency_to_csv(&latency, output_file, duration)?;
    Ok(())
}

fn write_latency_to_csv(
    latency: &[LatencyEntry],
    output_file: &str,
    duration: f64,
) -> io::Result<()> {
    let mut file = File::create(output_file)?;
    writeln!(
        file,
        "id,latency,processing_delay,jitter_buffer_delay,jitter_buffer_target_delay,jitter_buffer_minimum_delay,frames_per_second,freeze_count,total_bytes,dropped_frames,duration,cpu_usage"
    )?;
    for entry in latency {
        if entry.receive_timestamp == 0 || entry.rtc_stats.is_none() {
            continue;
        }
        let stats = entry.rtc_stats.unwrap();
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            entry.id,
            entry.receive_timestamp - entry.timestamp,
            stats.processing_delay,
            stats.jitter_buffer_delay,
            stats.jitter_buffer_target_delay,
            stats.jitter_buffer_minimum_delay,
            stats.frames_per_second,
            stats.freeze_count,
            stats.total_bytes,
            stats.dropped_frames,
            duration,
            entry.cpu_usage,
        )?;
    }
    Ok(())
}

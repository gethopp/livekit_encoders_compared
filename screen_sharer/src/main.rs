use clap::{value_parser, Arg, Command};
use livekit::options::{TrackPublishOptions, VideoCodec, VideoEncoding};
use livekit::prelude::*;
use livekit::track::{LocalTrack, LocalVideoTrack, TrackSource};
use livekit::webrtc::prelude::RtcVideoSource;
use screen_sharer::{handle_room_events, ScreenSharer};
use std::env;

#[derive(Debug, Clone)]
enum Resolution {
    HD1080,
    QHD1440,
    HD720,
    UHD2160,
}

impl Resolution {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Resolution::HD1080 => (1920, 1080),
            Resolution::QHD1440 => (2560, 1440),
            Resolution::HD720 => (1280, 720),
            Resolution::UHD2160 => (4096, 2160),
        }
    }
}

impl std::str::FromStr for Resolution {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "1080p" => Ok(Resolution::HD1080),
            "1440p" => Ok(Resolution::QHD1440),
            "720p" => Ok(Resolution::HD720),
            "4K" => Ok(Resolution::UHD2160),
            _ => Err(format!("Invalid resolution: {}. Use '1080p', '1440p', '4K', or '720p'", s)),
        }
    }
}

fn parse_video_codec(s: &str) -> Result<VideoCodec, String> {
    match s.to_uppercase().as_str() {
        "VP8" => Ok(VideoCodec::VP8),
        "VP9" => Ok(VideoCodec::VP9),
        "H264" => Ok(VideoCodec::H264),
        "AV1" => Ok(VideoCodec::AV1),
        _ => Err(format!("Invalid codec: {}. Use VP8, VP9, H264, or AV1", s)),
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let matches = Command::new("Screen Sharer")
        .version("1.0")
        .about("LiveKit screen sharing application")
        .arg(
            Arg::new("resolution")
                .long("res")
                .short('r')
                .help("Screen resolution (1080p or 1440p)")
                .value_parser(value_parser!(Resolution))
                .default_value("1080p")
        )
        .arg(
            Arg::new("duration")
                .long("duration")
                .short('d')
                .help("Duration in seconds")
                .value_parser(value_parser!(u64))
                .default_value("60")
        )
        .arg(
            Arg::new("codec")
                .long("codec")
                .short('c')
                .help("Video codec (VP8, VP9, H264, AV1)")
                .value_parser(parse_video_codec)
                .default_value("VP9")
        )
        .arg(
            Arg::new("bitrate")
                .long("bitrate")
                .short('b')
                .help("Bitrate in kbps (will be multiplied by 1000)")
                .value_parser(value_parser!(u64))
                .default_value("4000")
        )
        .arg(
            Arg::new("source_index")
                .long("source")
                .short('s')
                .help("Screen source index")
                .value_parser(value_parser!(u32))
                .default_value("0")
        )
        .arg(
            Arg::new("fps")
                .long("fps")
                .short('f')
                .help("Frames per second (default is 30)")
                .value_parser(value_parser!(u32))
                .default_value("30")
        )
        .arg(
            Arg::new("name")
                .long("name")
                .short('n')
                .help("Name for log file")
                .value_parser(value_parser!(String))
                .default_value("test")
        )
        .arg(
            Arg::new("simulcast")
                .long("simulcast")
                .help("Enable simulcast")
                .action(clap::ArgAction::SetTrue)
        )
        .get_matches();

    let resolution = matches.get_one::<Resolution>("resolution").unwrap();
    let duration = *matches.get_one::<u64>("duration").unwrap();
    let codec = matches.get_one::<VideoCodec>("codec").unwrap().clone();
    let bitrate = *matches.get_one::<u64>("bitrate").unwrap();
    let source_index = *matches.get_one::<u32>("source_index").unwrap();
    let fps = *matches.get_one::<u32>("fps").unwrap();
    let name = matches.get_one::<String>("name").unwrap();
    let simulcast = matches.get_flag("simulcast");

    let (width, height) = resolution.dimensions();

    let url = env::var("LIVEKIT_URL").expect("LIVEKIT_URL environment variable not set");
    let token = env::var("LIVEKIT_TOKEN").expect("LIVEKIT_TOKEN environment variable not set");

    let (room, mut rx) = Room::connect(&url, &token, RoomOptions::default())
        .await
        .unwrap();
    println!("Connected to room: {}", room.name());
    println!("Configuration: {}x{} @ {} fps, {} codec, {} kbps, simulcast: {}",
             width, height, fps, format!("{:?}", codec), bitrate,
             if simulcast { "enabled" } else { "disabled" });

    let mut screen_sharer = ScreenSharer::new(width, height, source_index).unwrap();

    let track = LocalVideoTrack::create_video_track(
        "screen_share",
        RtcVideoSource::Native(screen_sharer.buffer_source()),
    );

    let res = room
        .local_participant()
        .publish_track(
            LocalTrack::Video(track),
            TrackPublishOptions {
                source: TrackSource::Screenshare,
                video_codec: codec,
                video_encoding: Some(VideoEncoding {
                    max_bitrate: bitrate * 1000,
                    max_framerate: fps as f64,
                }),
                simulcast,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    handle_room_events(rx, screen_sharer.watermark_count());

    screen_sharer.start_capture(room);
    std::thread::sleep(std::time::Duration::from_secs(duration));
    screen_sharer.stop_capture(
        &format!("{:?}", codec),
        &format!("{}p", if height == 1080 { "1080" } else if height == 1440 { "1440" } else { "720" }),
        bitrate,
        &name,
    );
    /* Wait for the logs to be written. */
    std::thread::sleep(std::time::Duration::from_secs(5));
}

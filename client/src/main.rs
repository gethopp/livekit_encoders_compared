use client::end_to_end_latency;
use clap::Parser;
use livekit::prelude::*;
use std::env;

#[derive(Parser)]
#[command(name = "livekit-client")]
#[command(about = "LiveKit client for end-to-end latency measurement")]
struct Args {
    /// Output file path for latency measurements
    #[arg(short, long)]
    output_file: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let args = Args::parse();
    let url = env::var("LIVEKIT_URL").expect("LIVEKIT_URL environment variable not set");
    let token = env::var("LIVEKIT_TOKEN").expect("LIVEKIT_TOKEN environment variable not set");

    let (room, mut rx) = Room::connect(&url, &token, RoomOptions::default())
        .await
        .unwrap();
    while let Some(msg) = rx.recv().await {
        match msg {
            RoomEvent::TrackSubscribed {
                track,
                publication: _,
                participant: _,
            } => {
                if let RemoteTrack::Video(track) = track {
                    end_to_end_latency(room, track, &args.output_file).await.unwrap();
                    break;
                }
            }
            _ => {}
        }
    }
}

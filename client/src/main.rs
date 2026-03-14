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

    /* Check for already-subscribed video tracks. */
    let existing_track = room.remote_participants().iter().find_map(|(_, p)| {
        p.track_publications().iter().find_map(|(_, pub_)| {
            if pub_.source() == TrackSource::Screenshare {
                if let Some(RemoteTrack::Video(track)) = pub_.track() {
                    return Some(track);
                }
            }
            None
        })
    });

    if let Some(track) = existing_track {
        log::info!("Found existing video track, starting measurement");
        end_to_end_latency(room, track, &args.output_file).await.unwrap();
    } else {
        while let Some(msg) = rx.recv().await {
            match msg {
                RoomEvent::TrackSubscribed {
                    track,
                    publication,
                    participant,
                } => {
                    log::info!(
                        "TrackSubscribed: participant={} (id={}), track sid={}, source={:?}, kind={:?}, mime_type={}",
                        participant.name(),
                        participant.identity(),
                        publication.sid(),
                        publication.source(),
                        publication.kind(),
                        publication.mime_type(),
                    );
                    log::info!("Track: {:?}", track);
                    if let RemoteTrack::Video(track) = track {
                        if publication.source() == TrackSource::Screenshare {
                            log::info!("Starting measurement on screenshare track");
                            end_to_end_latency(room, track, &args.output_file).await.unwrap();
                            break;
                        } else {
                            log::info!("Skipping non-screenshare video track (source={:?})", publication.source());
                        }
                    }
                }
                other => {
                    log::info!("RoomEvent: {:?}", other);
                }
            }
        }
    }
}

# LiveKit Encoder Comparison

A Rust-based screen sharing application for testing and comparing different video codecs with LiveKit.

## Prerequisites

- Rust (latest stable version)
- Cargo
- LiveKit server instance
- LiveKit access token

## Environment Setup

Before running the screen_sharer, you need to set the following environment variables:

```bash
export LIVEKIT_URL="your_livekit_server_url"
export LIVEKIT_TOKEN="your_livekit_access_token"
```

**Important**: When testing both the screen_sharer and client together, ensure that both tokens are created for the same LiveKit room. You can generate tokens using the [LiveKit CLI](https://docs.livekit.io/home/cli/cli-setup/) or programmatically using the LiveKit SDK.

## Screen Sharer

The `screen_sharer` application captures your screen and streams it to a LiveKit room with configurable video encoding parameters.

### Building

The encoder uses a LiveKit Rust SDK [fork](https://github.com/iparaskev/rust-sdks/tree/add_desktop_capturer) that exposes libwebrtc's `DesktopCapturer`. Before building the screen_sharer, you need to:

1. Build the updated libwebrtc library from our fork (which contains our patches)
2. Point to it using the `LK_CUSTOM_WEBRTC` environment variable

You can find instructions for building the custom libwebrtc in the [LiveKit Rust SDK documentation](https://github.com/livekit/rust-sdks/tree/main/webrtc-sys/libwebrtc#readme).

Once the custom libwebrtc is built, navigate to the screen_sharer directory and build the application:

```bash
export LK_CUSTOM_WEBRTC="/path/to/your/custom/libwebrtc"
cd screen_sharer
cargo build --release
```

### Running

Basic usage:

```bash
cargo run
```

### Command Line Options

The screen_sharer supports various configuration options:

| Option | Short | Description | Default | Available Values |
|--------|-------|-------------|---------|------------------|
| `--resolution` | `-r` | Screen resolution | `1080p` | `720p`, `1080p`, `1440p` |
| `--duration` | `-d` | Recording duration in seconds | `60` | Any positive integer |
| `--codec` | `-c` | Video codec | `VP9` | `VP8`, `VP9`, `H264`, `AV1` |
| `--bitrate` | `-b` | Bitrate in kbps | `4000` | Any positive integer |
| `--source` | `-s` | Screen source index | `0` | Any valid screen index |
| `--fps` | `-f` | Frames per second | `30` | Any positive integer |
| `--name` | `-n` | Name for log file | `test` | Any string |
| `--simulcast` | | Enable simulcast | `false` | Flag (no value needed) |

### Examples

#### Basic screen sharing with default settings:
```bash
cargo run
```

#### Record in 1440p with H264 codec for 2 minutes:
```bash
cargo run -- --resolution 1440p --codec H264 --duration 120
```

#### High bitrate VP9 encoding with simulcast:
```bash
cargo run -- --codec VP9 --bitrate 8000 --simulcast --name high_quality_test
```

#### AV1 encoding test:
```bash
cargo run -- --codec AV1 --bitrate 2000 --duration 180 --name av1_test
```

### Output

The application will:
1. Connect to the specified LiveKit room
2. Start screen capture and streaming
3. Display configuration information
4. Generate CSV files with performance metrics
5. Save logs with the specified name

Generated files include CPU usage data and encoding performance metrics saved in the `screen_sharer` directory.

## Client Application

The `client` application is designed to measure end-to-end latency by connecting to LiveKit rooms and receiving video streams. Unlike the screen_sharer, the client uses the standard LiveKit Rust SDK and doesn't require the custom fork.

### Building the Client

Navigate to the client directory and build:

```bash
cd client
cargo build --release
```

### Running the Client

The client requires the same environment variables as the screen_sharer:

```bash
export LIVEKIT_URL="your_livekit_server_url"
export LIVEKIT_TOKEN="your_livekit_access_token"
```

Run the client with an output file for latency measurements:

```bash
cargo run -- --output-file latency_results.csv
```

### Command Line Options

| Option | Short | Description | Required |
|--------|-------|-------------|----------|
| `--output-file` | `-o` | Output file path for latency measurements | Yes |

## Usage Example

To measure end-to-end latency during a screen sharing session:

1. Start the screen_sharer in one terminal:
```bash
cd screen_sharer
cargo run -- --codec VP9 --duration 300 --name latency_test
```

2. Start the client in another terminal:
```bash
cd client
cargo run -- --output-file latency_vp9_test.csv
```

The client will automatically connect to the same LiveKit room and begin measuring latency as soon as it receives video frames from the screen_sharer.
use crabgrab::feature::bitmap::{FrameBitmap, VideoFrameBitmap};
use crabgrab::platform::macos::MacosCaptureConfigExt;
use crabgrab::prelude::*;
use std::process::Stdio;
use tokio::sync::mpsc;

use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};

const FRAME_RATE: u64 = 30;
const CAPTURE_DURATION: Duration = Duration::from_secs(20);
// Buffer used to pass video frames from the capture stream to the ffmpeg process
const BUFFER_SIZE: usize = 10000;

async fn main_async() {
    let token = match CaptureStream::test_access(false) {
        Some(token) => token,
        None => CaptureStream::request_access(false)
            .await
            .expect("Expected capture access"),
    };

    let filter = CapturableContentFilter::DISPLAYS;
    let content = CapturableContent::new(filter).await.unwrap();
    let display = content.displays().next().expect("No display found");
    let config = CaptureConfig::with_display(display.clone(), CapturePixelFormat::Bgra8888)
        .with_buffer_count(128)
        .with_maximum_fps(Some(FRAME_RATE as f32));
    let (tx, mut rx) = mpsc::channel(BUFFER_SIZE);

    tokio::spawn(async move {
        let mut _stream = CaptureStream::new(token, config, move |result| {
            if let Ok(StreamEvent::Video(frame)) = result {
                println!(
                    "Received frame {} at: {:?}",
                    frame.frame_id(),
                    std::time::Instant::now()
                );
                if let Ok(FrameBitmap::BgraUnorm8x4(image_bitmap_bgra8888)) = frame.get_bitmap() {
                    let flat_data: Vec<u8> = image_bitmap_bgra8888
                        .data
                        .iter()
                        .flat_map(|&[b, g, r, a]| vec![b, g, r, a])
                        .collect();
                    let _ = tx.blocking_send(flat_data);
                }
            }
        })
        .expect("Failed to create capture stream");
        std::thread::sleep(CAPTURE_DURATION);

        println!("Stopping capture stream...");

        // Stop the stream after capturing for the defined duration
        _stream.stop().expect("Failed to stop capture stream");
    });

    // wait for capture to start
    std::thread::sleep(Duration::from_secs(1));
    // Now process the frames and send them to ffmpeg
    let resolution = format!(
        "{}x{}",
        display.rect().size.width,
        display.rect().size.height
    );
    let frame_rate_str = FRAME_RATE.to_string();

    let ffmpeg_command = vec![
        "-f",
        "rawvideo",
        "-pix_fmt",
        "bgra",
        "-s",
        &resolution,
        "-r",
        &frame_rate_str,
        "-i",
        "pipe:0",
        "-an",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "output.mp4",
    ];

    let ffmpeg_binary_path = "ffmpeg"; // Assuming ffmpeg is in your PATH
    let ffmpeg_command = ffmpeg_command.clone();
    let mut ffmpeg_process = start_ffmpeg_sidecar(ffmpeg_binary_path, &ffmpeg_command).await;

    while let Some(flat_data) = rx.recv().await {
        // println!(
        //     "ffmpeg received frame {} at {:?}",
        //     frame.frame_id(),
        //     std::time::Instant::now()
        // );
        // if let Ok(FrameBitmap::BgraUnorm8x4(image_bitmap_bgra8888)) = frame.get_bitmap() {
        //     let flat_data: Vec<u8> = image_bitmap_bgra8888
        //         .data
        //         .iter()
        //         .flat_map(|&[b, g, r, a]| vec![b, g, r, a])
        //         .collect();

        ffmpeg_process
            .stdin
            .as_mut()
            .unwrap()
            .write_all(&flat_data)
            .await
            .expect("Failed to write frame to FFmpeg process");
        // }
    }

    ffmpeg_process
        .stdin
        .as_mut()
        .unwrap()
        .shutdown()
        .await
        .expect("Failed to shutdown stdin");
    ffmpeg_process.wait().await.expect("FFmpeg process failed");

    println!("Video recording completed");
}

async fn start_ffmpeg_sidecar(ffmpeg_binary_path: &str, args: &[&str]) -> Child {
    Command::new(ffmpeg_binary_path)
        .args(args)
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn FFmpeg process")
}

#[tokio::main]
async fn main() {
    main_async().await;
}

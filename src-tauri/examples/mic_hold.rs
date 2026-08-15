//! Empirical check for the idle-release fix: does dropping a cpal input
//! stream actually release the CoreAudio power assertion that keeps the
//! machine awake? Run it and watch `pmset -g assertions` for this pid.
//! Holds the mic for 20s, drops the stream, then idles for 20s more.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() {
    println!("pid={}", std::process::id());
    let host = cpal::default_host();
    let device = host.default_input_device().expect("no input device");
    let config = device.default_input_config().expect("no input config");
    let stream = device
        .build_input_stream(
            &config.clone().into(),
            move |_data: &[f32], _: &cpal::InputCallbackInfo| {},
            |e| eprintln!("stream error: {e}"),
            None,
        )
        .expect("build stream");
    stream.play().expect("play");
    println!("STREAM_OPEN");
    std::thread::sleep(std::time::Duration::from_secs(20));
    drop(stream);
    println!("STREAM_DROPPED");
    std::thread::sleep(std::time::Duration::from_secs(20));
    println!("DONE");
}

use std::error::Error;
use std::io::BufReader;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let stream_handle = rodio::DeviceSinkBuilder::open_default_sink()?;
    let mixer = stream_handle.mixer();

    // Play a WAV file.
    let file = std::fs::File::open("assets/audio/swoof.wav")?;
    let player = rodio::play(mixer, BufReader::new(file))?;
    player.set_volume(0.2);

    println!("Started beep1");
    thread::sleep(Duration::from_millis(1500));

    Ok(())
}

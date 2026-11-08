//! Passive, finite VNC profile. Password comes from stdin and is never written to disk.
#[allow(dead_code)]
#[path = "../src/rfb.rs"]
mod rfb;
use std::{
    io, thread,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::args().nth(1).ok_or("host required")?;
    let seconds: u64 = std::env::args().nth(2).unwrap_or("15".into()).parse()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim_end_matches(['\r', '\n']).to_owned();
    let session = rfb::Session::start(
        rfb::Options {
            host,
            port: 5900,
            password,
            quality: rfb::Quality::Balanced,
            shared: true,
            listen: false,
            listen_port: 5500,
            clipboard: false,
        },
        eframe::egui::Context::default(),
    );
    let started = Instant::now();
    let mut connected_at = None;
    let mut uploads = 0_u64;
    let mut upload_pixels = 0_u64;
    let mut timings = vec![];
    loop {
        let tick = Instant::now();
        let (connected, failed, status, patches) = {
            let mut state = session.state.lock().unwrap();
            (
                state.connected,
                state.failed,
                state.status.clone(),
                state.patches.drain(..).collect::<Vec<_>>(),
            )
        };
        if failed {
            return Err(status.into());
        }
        if connected && connected_at.is_none() {
            connected_at = Some(Instant::now());
        }
        for patch in patches {
            let t = Instant::now();
            let image = patch.into_image();
            upload_pixels += image.pixels.len() as u64;
            std::hint::black_box(image);
            timings.push(t.elapsed().as_secs_f64() * 1000.);
            uploads += 1;
        }
        if connected_at.is_some_and(|t: Instant| t.elapsed() >= Duration::from_secs(seconds)) {
            break;
        }
        if connected_at.is_none() && started.elapsed() > Duration::from_secs(20) {
            return Err("connection timeout".into());
        }
        thread::sleep(Duration::from_secs_f64(1. / 60.).saturating_sub(tick.elapsed()));
    }
    timings.sort_by(f64::total_cmp);
    let elapsed = connected_at.unwrap().elapsed().as_secs_f64();
    let state = session.state.lock().unwrap();
    let result = serde_json::json!({
        "seconds": elapsed, "width": state.width, "height": state.height,
        "server_updates": state.frames, "updates_per_second": state.frames as f64 / elapsed,
        "encoding": state.encoding, "uploaded_patches": uploads,
        "continuous_updates": state.continuous,
        "last_decode_ms": state.decode_ms,
        "coalesced_updates": state.coalesced_updates,
        "upload_megapixels": upload_pixels as f64 / 1_000_000.,
        "ui_image_prepare_p95_ms": timings.get((timings.len() * 95 / 100).min(timings.len().saturating_sub(1))),
        "note": "Passive real-server sample; no input, forced refresh, or GPU presentation measurement."
    });
    drop(state);
    session.stop();
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

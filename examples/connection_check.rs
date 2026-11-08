//! Finite loopback fixture: launches the real viewer, sends RFB frames, waits for its PNG, exits.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn read(r: &mut impl Read, n: usize) -> std::io::Result<Vec<u8>> {
    let mut a = vec![0; n];
    r.read_exact(&mut a)?;
    Ok(a)
}
fn rectangle(x: u16, y: u16, w: u16, h: u16, color: [u8; 3]) -> Vec<u8> {
    let mut a = vec![];
    for n in [x, y, w, h] {
        a.extend(n.to_be_bytes());
    }
    a.extend(7i32.to_be_bytes());
    a.push(0x80);
    a.extend(color);
    a
}
fn main() -> std::io::Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compact = std::env::args().any(|arg| arg == "--compact");
    let screenshot = root.join(if compact {
        "shots/connected-compact.png"
    } else {
        "shots/connected.png"
    });
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    let viewer_path = std::env::current_exe()?
        .parent()
        .and_then(|dir| dir.parent())
        .ok_or_else(|| std::io::Error::other("Cannot locate Cargo output directory"))?
        .join(format!("deskora{}", std::env::consts::EXE_SUFFIX));
    let mut viewer = Command::new(viewer_path);
    if compact {
        viewer.args(["--screenshot-size", "580", "520"]);
    }
    let mut child = viewer
        .args([
            "--connect",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--screenshot",
        ])
        .arg(&screenshot)
        .spawn()?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut s, _) = listener.accept()?;
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        s.write_all(b"RFB 003.008\n")?;
        read(&mut s, 12)?;
        s.write_all(&[1, 1])?;
        read(&mut s, 1)?;
        s.write_all(&[0; 4])?;
        read(&mut s, 1)?;
        s.write_all(&800u16.to_be_bytes())?;
        s.write_all(&450u16.to_be_bytes())?;
        s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0])?;
        s.write_all(&17u32.to_be_bytes())?;
        s.write_all(b"Verifica VNC Rust")?;
        read(&mut s, 20)?;
        let a = read(&mut s, 4)?;
        read(&mut s, u16::from_be_bytes([a[2], a[3]]) as usize * 4)?;
        let mut frame = 0;
        while read(&mut s, 10).is_ok() {
            let rects = if frame == 0 {
                vec![
                    rectangle(0, 0, 800, 450, [28, 77, 79]),
                    rectangle(120, 65, 560, 320, [246, 248, 249]),
                    rectangle(120, 65, 560, 35, [225, 232, 232]),
                    rectangle(150, 128, 210, 24, [17, 112, 104]),
                    rectangle(150, 173, 445, 9, [176, 190, 192]),
                    rectangle(150, 198, 390, 9, [176, 190, 192]),
                    rectangle(150, 223, 415, 9, [176, 190, 192]),
                    rectangle(150, 287, 180, 49, [17, 112, 104]),
                    rectangle(355, 287, 180, 49, [220, 231, 231]),
                ]
            } else {
                vec![rectangle(
                    700,
                    400,
                    80,
                    30,
                    if frame % 2 == 0 {
                        [72, 180, 156]
                    } else {
                        [50, 140, 130]
                    },
                )]
            };
            let mut msg = vec![0, 0];
            msg.extend((rects.len() as u16).to_be_bytes());
            for r in rects {
                msg.extend(r);
            }
            s.write_all(&msg)?;
            frame += 1;
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if Instant::now() > deadline {
            child.kill()?;
            return Err(std::io::Error::other("Screenshot timeout"));
        }
        thread::sleep(Duration::from_millis(20));
    };
    let _ = server.join();
    assert!(status.success());
    assert!(screenshot.metadata()?.len() > 10000);
    println!(
        "Loopback VNC: real viewer connected, compressed frames displayed, screenshot saved, clean exit."
    );
    Ok(())
}

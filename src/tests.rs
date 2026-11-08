use super::*;
use des::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use flate2::{Compress, Compression, FlushCompress};
use std::{
    io::{Cursor, Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

fn pack(c: &mut Compress, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0; data.len() * 2 + 1024];
    let before = c.total_out();
    c.compress(data, &mut out, FlushCompress::Sync).unwrap();
    out.truncate((c.total_out() - before) as usize);
    out
}
fn compact(n: usize) -> Vec<u8> {
    if n < 128 {
        vec![n as u8]
    } else if n < 16384 {
        vec![(n as u8 & 127) | 128, (n >> 7) as u8]
    } else {
        vec![
            (n as u8 & 127) | 128,
            ((n >> 7) as u8 & 127) | 128,
            (n >> 14) as u8,
        ]
    }
}
fn rgb(d: &rfb::Decoder, x: usize, y: usize) -> [u8; 4] {
    d.pixels[(y * d.w + x) * 4..(y * d.w + x + 1) * 4]
        .try_into()
        .unwrap()
}
#[test]
fn address_rules() {
    assert_eq!(
        rfb::parse_address(" 192.168.1.10 ", 5900).unwrap(),
        ("192.168.1.10".into(), 5900)
    );
    assert_eq!(
        rfb::parse_address("pc::5902", 5900).unwrap(),
        ("pc".into(), 5902)
    );
    assert_eq!(rfb::parse_address("pc:2", 5900).unwrap().1, 5902);
    assert_eq!(rfb::parse_address("pc:0", 5900).unwrap().1, 5900);
    assert_eq!(rfb::parse_address("pc:2", 6000).unwrap().1, 6002);
    assert_eq!(rfb::parse_address("pc:0", 6000).unwrap().1, 6000);
    assert_eq!(rfb::parse_address("pc:99", 6000).unwrap().1, 6099);
    assert_eq!(rfb::parse_address("pc:100", 6000).unwrap().1, 100);
    assert_eq!(rfb::parse_address("pc::2", 6000).unwrap().1, 2);
    assert_eq!(rfb::parse_address("pc:35", 65500).unwrap().1, 65535);
    assert!(rfb::parse_address("pc:36", 65500).is_err());
    assert!(rfb::parse_address("pc:0", 0).is_err());
    assert_eq!(
        rfb::parse_address("[::1]:5999", 5900).unwrap(),
        ("::1".into(), 5999)
    );
    assert_eq!(rfb::parse_address("::1", 5902).unwrap().1, 5902);
    for s in ["", "http://pc", "pc::0", "pc:99999", "bad host", "[::1"] {
        assert!(rfb::parse_address(s, 5900).is_err(), "{s}");
    }
}
#[test]
fn raw_and_overlapping_copyrect() {
    let mut d = rfb::Decoder::new(4, 1).unwrap();
    d.rectangle(
        &mut Cursor::new([1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0]),
        0,
        0,
        4,
        1,
        0,
    )
    .unwrap();
    assert_eq!(rgb(&d, 0, 0), [3, 2, 1, 255]);
    d.rectangle(&mut Cursor::new([0, 0, 0, 0]), 1, 0, 3, 1, 1)
        .unwrap();
    assert_eq!(rgb(&d, 3, 0), [9, 8, 7, 255]);
    assert_eq!(rgb(&d, 2, 0), [6, 5, 4, 255]);
}
#[test]
fn tight_fill_palette_and_gradient() {
    let mut d = rfb::Decoder::new(9, 2).unwrap();
    d.rectangle(&mut Cursor::new([0x80, 10, 20, 30]), 0, 0, 9, 2, 7)
        .unwrap();
    assert_eq!(rgb(&d, 8, 1), [10, 20, 30, 255]);
    let palette = [
        0x40, 1, 1, 255, 0, 0, 0, 255, 0, 0b10101010, 0b10000000, 0b01010101, 0,
    ];
    d.rectangle(&mut Cursor::new(palette), 0, 0, 9, 2, 7)
        .unwrap();
    assert_eq!(rgb(&d, 8, 0), [0, 255, 0, 255]);
    assert_eq!(rgb(&d, 8, 1), [255, 0, 0, 255]);
    d.rectangle(
        &mut Cursor::new([0x40, 2, 100, 50, 25, 10, 10, 10]),
        0,
        0,
        2,
        1,
        7,
    )
    .unwrap();
    assert_eq!(rgb(&d, 1, 0), [110, 60, 35, 255]);
}
#[test]
fn tight_preserves_zlib_stream_between_rectangles() {
    let mut d = rfb::Decoder::new(4, 2).unwrap();
    let mut c = Compress::new(Compression::fast(), true);
    for row in 0..2 {
        let data = [10 + row as u8, 20, 30].repeat(4);
        let compressed = pack(&mut c, &data);
        let mut input = vec![if row == 0 { 1 } else { 0 }];
        input.extend(compact(compressed.len()));
        input.extend(compressed);
        d.rectangle(&mut Cursor::new(input), 0, row, 4, 1, 7)
            .unwrap();
        assert_eq!(rgb(&d, 3, row), [10 + row as u8, 20, 30, 255]);
    }
}
#[test]
fn zrle_raw_packed_palette_and_runs() {
    for (data, expected) in [
        (
            vec![0, 30, 20, 10, 60, 50, 40, 90, 80, 70, 120, 110, 100],
            [100, 110, 120, 255],
        ),
        (vec![2, 0, 0, 255, 0, 255, 0, 0b01010000], [0, 255, 0, 255]),
        (vec![128, 10, 20, 30, 3], [30, 20, 10, 255]),
        (vec![130, 0, 0, 255, 0, 255, 0, 129, 3], [0, 255, 0, 255]),
    ] {
        let packed = pack(&mut Compress::new(Compression::fast(), true), &data);
        let mut input = (packed.len() as u32).to_be_bytes().to_vec();
        input.extend(packed);
        let mut d = rfb::Decoder::new(4, 2).unwrap();
        d.rectangle(&mut Cursor::new(input), 0, 0, 4, 1, 16)
            .unwrap();
        assert_eq!(rgb(&d, 3, 0), expected);
    }
}
#[test]
fn rejects_invalid_rectangles_and_palette_indices() {
    let mut d = rfb::Decoder::new(4, 4).unwrap();
    assert!(d.rectangle(&mut Cursor::new([]), 3, 3, 4, 4, 0).is_err());
    assert!(
        d.rectangle(&mut Cursor::new([0x40, 1, 0, 1, 2, 3, 7]), 0, 0, 1, 1, 7)
            .is_err()
    );
    assert!(
        d.rectangle(&mut Cursor::new([0xf0]), 0, 0, 1, 1, 7)
            .is_err()
    );
    assert!(rfb::Decoder::new(65535, 65535).is_err());
}
#[test]
fn vnc_des_challenge_uses_reversed_key_and_eight_byte_limit() {
    let challenge = std::array::from_fn(|i| i as u8);
    let response = rfb::vnc_response("password ignored", challenge);
    assert_eq!(response, rfb::vnc_response("password", challenge));
    let cipher = des::Des::new(GenericArray::from_slice(&[
        0x0e, 0x86, 0xce, 0xce, 0xee, 0xf6, 0x4e, 0x26,
    ]));
    let mut decrypted = response;
    for block in decrypted.as_chunks_mut::<8>().0 {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    assert_eq!(decrypted, challenge);
}

fn read_n(s: &mut impl Read, n: usize) -> Vec<u8> {
    let mut v = vec![0; n];
    s.read_exact(&mut v).unwrap();
    v
}
fn server_init(s: &mut impl Write) {
    s.write_all(&8u16.to_be_bytes()).unwrap();
    s.write_all(&4u16.to_be_bytes()).unwrap();
    s.write_all(&[32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0])
        .unwrap();
    s.write_all(&7u32.to_be_bytes()).unwrap();
    s.write_all(b"Test PC").unwrap();
}
fn rect_header(x: u16, y: u16, w: u16, h: u16, e: i32) -> Vec<u8> {
    let mut v = Vec::new();
    for n in [x, y, w, h] {
        v.extend(n.to_be_bytes());
    }
    v.extend(e.to_be_bytes());
    v
}

#[test]
fn live_rfb_auth_frame_clipboard_input_and_cancellation() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        s.write_all(b"RFB 003.008\n").unwrap();
        assert_eq!(read_n(&mut s, 12), b"RFB 003.008\n");
        s.write_all(&[1, 2]).unwrap();
        assert_eq!(read_n(&mut s, 1), [2]);
        let challenge: [u8; 16] = std::array::from_fn(|i| i as u8);
        s.write_all(&challenge).unwrap();
        let mut response = read_n(&mut s, 16);
        let cipher = des::Des::new(GenericArray::from_slice(&[
            0x0e, 0x86, 0xce, 0xce, 0xee, 0xf6, 0x4e, 0x26,
        ]));
        for block in response.as_chunks_mut::<8>().0 {
            cipher.decrypt_block(GenericArray::from_mut_slice(block));
        }
        assert_eq!(response, challenge);
        s.write_all(&0u32.to_be_bytes()).unwrap();
        assert_eq!(read_n(&mut s, 1), [1]);
        server_init(&mut s);
        assert_eq!(
            read_n(&mut s, 20),
            [
                0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0
            ]
        );
        let enc = read_n(&mut s, 4);
        let n = u16::from_be_bytes([enc[2], enc[3]]) as usize;
        let encodings = read_n(&mut s, n * 4);
        assert!(
            encodings
                .as_chunks::<4>()
                .0
                .iter()
                .any(|e| *e == 7i32.to_be_bytes())
        );
        assert!(
            !encodings
                .as_chunks::<4>()
                .0
                .iter()
                .any(|e| *e == (-26i32).to_be_bytes())
        );
        assert_eq!(read_n(&mut s, 10), rfb::request(8, 4, false));
        let mut update = vec![0, 0, 0, 2];
        update.extend(rect_header(0, 0, 2, 2, 7));
        update.extend([0x80, 210, 140, 60]);
        update.extend(rect_header(2, 0, 2, 2, 1));
        update.extend([0, 0, 0, 0]);
        s.write_all(&update).unwrap();
        s.write_all(&[3, 0, 0, 0, 0, 0, 0, 4, b'C', b'a', b'f', 0xe9])
            .unwrap();
        let (mut pointer, mut key, mut clipboard) = (false, false, false);
        while !(pointer && key && clipboard) {
            match read_n(&mut s, 1)[0] {
                3 => {
                    read_n(&mut s, 9);
                }
                4 => {
                    let a = read_n(&mut s, 7);
                    key |= a == [1, 0, 0, 0, 0, 0, b'a'];
                }
                5 => {
                    let a = read_n(&mut s, 5);
                    pointer |= a == [1, 0, 2, 0, 1];
                }
                6 => {
                    read_n(&mut s, 3);
                    let n = u32::from_be_bytes(read_n(&mut s, 4).try_into().unwrap()) as usize;
                    clipboard = read_n(&mut s, n) == b"local text";
                }
                other => panic!("unexpected {other}"),
            }
        }
        done_tx.send(()).unwrap();
        let mut a = [0];
        let _ = s.read(&mut a);
    });
    let session = Session::start(
        rfb::Options {
            host: "127.0.0.1".into(),
            port,
            password: "password".into(),
            quality: Quality::Sharp,
            shared: true,
            listen: false,
            listen_port: 5500,
            clipboard: true,
        },
        egui::Context::default(),
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let s = session.state.lock().unwrap();
        assert!(!s.failed, "{}", s.status);
        if s.frames == 1 && s.clipboard.is_some() {
            assert_eq!(s.name, "Test PC");
            assert_eq!(s.clipboard.as_deref(), Some("Café"));
            let p = &s.patches[0];
            assert_eq!(p.w, 4);
            assert_eq!(p.pixels, vec![egui::Color32::from_rgb(210, 140, 60); 8]);
            break;
        }
        drop(s);
        assert!(Instant::now() < deadline, "frame timeout");
        thread::sleep(Duration::from_millis(5));
    }
    session.send(rfb::pointer(2, 1, 1));
    session.send(rfb::key(b'a' as u32, true));
    session.send(rfb::cut_text("local text"));
    done_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    session.stop();
    server.join().unwrap();
}
#[test]
fn failed_authentication_is_reported_without_a_frame() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.write_all(b"RFB 003.008\n").unwrap();
        read_n(&mut s, 12);
        s.write_all(&[1, 2]).unwrap();
        read_n(&mut s, 1);
        s.write_all(&[0; 16]).unwrap();
        read_n(&mut s, 16);
        s.write_all(&[0, 0, 0, 1, 0, 0, 0, 3, b'b', b'a', b'd'])
            .unwrap();
    });
    let session = Session::start(
        rfb::Options {
            host: "127.0.0.1".into(),
            port,
            password: "wrong".into(),
            quality: Quality::Sharp,
            shared: true,
            listen: false,
            listen_port: 5500,
            clipboard: false,
        },
        egui::Context::default(),
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let s = session.state.lock().unwrap();
        if s.failed {
            assert!(s.status.contains("Accesso rifiutato"));
            assert_eq!(s.frames, 0);
            break;
        }
        drop(s);
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    session.stop();
    server.join().unwrap();
}
#[test]
fn listen_can_be_cancelled_and_releases_its_port() {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let session = Session::start(
        rfb::Options {
            host: String::new(),
            port: 5900,
            password: String::new(),
            quality: Quality::Sharp,
            shared: true,
            listen: true,
            listen_port: port,
            clipboard: false,
        },
        egui::Context::default(),
    );
    thread::sleep(Duration::from_millis(80));
    session.stop();
    thread::sleep(Duration::from_millis(100));
    assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
}

fn key_event(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}
#[test]
fn printable_key_is_held_and_released_after_shift_changes() {
    let mut kb = input::RemoteKeyboard::default();
    let shift = egui::Modifiers::SHIFT;
    let packets = kb.translate(
        &[
            key_event(egui::Key::A, true, shift),
            egui::Event::Text("A".into()),
        ],
        shift,
        true,
    );
    assert_eq!(packets, vec![rfb::key(0xffe1, true), rfb::key(65, true)]);
    let packets = kb.translate(
        &[key_event(egui::Key::A, false, egui::Modifiers::NONE)],
        egui::Modifiers::NONE,
        true,
    );
    assert!(packets.contains(&rfb::key(65, false)));
    assert!(packets.contains(&rfb::key(0xffe1, false)));
    assert!(kb.release().is_empty());
}
#[test]
fn repeat_and_focus_loss_do_not_leave_keys_pressed() {
    let mut kb = input::RemoteKeyboard::default();
    let events = [
        key_event(egui::Key::W, true, egui::Modifiers::NONE),
        egui::Event::Text("w".into()),
    ];
    assert_eq!(
        kb.translate(&events, egui::Modifiers::NONE, true),
        vec![rfb::key(119, true)]
    );
    assert_eq!(
        kb.translate(&events, egui::Modifiers::NONE, true),
        vec![rfb::key(119, true)]
    );
    assert_eq!(kb.release(), vec![rfb::key(119, false)]);
    assert!(kb.release().is_empty());
}
#[test]
fn clipboard_shortcuts_and_unicode_survive_native_event_translation() {
    let mut kb = input::RemoteKeyboard::default();
    for (event, letter) in [
        (egui::Event::Copy, b'c'),
        (egui::Event::Cut, b'x'),
        (egui::Event::Paste("secret local text".into()), b'v'),
    ] {
        let p = kb.translate(&[event], egui::Modifiers::NONE, false);
        assert_eq!(
            p,
            vec![
                rfb::key(0xffe3, true),
                rfb::key(letter as u32, true),
                rfb::key(letter as u32, false),
                rfb::key(0xffe3, false)
            ]
        );
        assert!(!p.iter().any(|p| p[0] == 6));
    }
    let p = kb.translate(
        &[egui::Event::Text("€".into())],
        egui::Modifiers::NONE,
        true,
    );
    assert_eq!(
        p,
        vec![rfb::key(0x010020ac, true), rfb::key(0x010020ac, false)]
    );
}
#[test]
fn compatible_versions_and_tight_security_negotiate() {
    let o = rfb::Options {
        host: "pc".into(),
        port: 5900,
        password: String::new(),
        quality: Quality::Balanced,
        shared: true,
        listen: false,
        listen_port: 5500,
        clipboard: true,
    };
    for (major, minor, version, tight) in [
        (3, 3, 3, false),
        (3, 4, 3, false),
        (3, 6, 3, false),
        (3, 7, 7, false),
        (3, 8, 8, false),
        (3, 8, 8, true),
        (3, 889, 8, false),
        (4, 0, 8, false),
        (999, 999, 8, false),
    ] {
        let mut wire = format!("RFB {major:03}.{minor:03}\n").into_bytes();
        if version == 3 {
            wire.extend(1u32.to_be_bytes());
        } else {
            wire.extend([1, if tight { 16 } else { 1 }]);
        }
        if tight {
            wire.extend(0u32.to_be_bytes());
            wire.extend(1u32.to_be_bytes());
            wire.extend(1u32.to_be_bytes());
            wire.extend(*b"STDVNOAUTH__");
        }
        if version == 8 {
            wire.extend(0u32.to_be_bytes());
        }
        server_init(&mut wire);
        if tight {
            wire.extend([0; 8]);
        }
        let mut reader = Cursor::new(wire);
        let mut sent = vec![];
        let (w, h, name) = rfb::handshake(&mut reader, &mut sent, &o).unwrap();
        assert_eq!((w, h, name), (8, 4, "Test PC".into()));
        assert_eq!(&sent[..12], format!("RFB 003.{version:03}\n").as_bytes());
        assert_eq!(reader.position() as usize, reader.get_ref().len());
    }
    for banner in [
        b"RFB 003,008\n",
        b"RFB +03.008\n",
        b"RFB 003.+08\n",
        b"RFB 003.008!",
        b"BAD 003.008\n",
        b"RFB 002.999\n",
        b"RFB 003.002\n",
    ] {
        let mut sent = vec![];
        assert!(rfb::handshake(&mut Cursor::new(banner), &mut sent, &o).is_err());
        assert!(
            sent.is_empty(),
            "Invalid banner must be rejected before replying"
        );
    }
}

#[test]
fn recovered_quality_rules_reach_the_wire() {
    for level in 0..=9 {
        assert_eq!(rfb::jpeg_encoding(level), Some(-32 + level));
        assert_eq!(rfb::compression_encoding(level), Some(-256 + level));
    }
    for level in [-2, -1, 10, 256] {
        assert_eq!(rfb::jpeg_encoding(level), None);
        assert_eq!(rfb::compression_encoding(level), None);
    }
    for (quality, expected) in [
        (Quality::Sharp, vec![]),
        (Quality::Balanced, vec![-26]),
        (Quality::Fluid, vec![-23, -255]),
    ] {
        let o = rfb::Options {
            host: "pc".into(),
            port: 5900,
            password: String::new(),
            quality,
            shared: true,
            listen: false,
            listen_port: 5500,
            clipboard: true,
        };
        let mut wire = b"RFB 003.008\n".to_vec();
        wire.extend([1, 1, 0, 0, 0, 0]);
        server_init(&mut wire);
        let mut sent = vec![];
        rfb::handshake(&mut Cursor::new(wire), &mut sent, &o).unwrap();
        // Banner 12, security selection 1, shared flag 1, pixel format 20.
        assert_eq!(&sent[34..36], &[2, 0]);
        let count = u16::from_be_bytes([sent[36], sent[37]]) as usize;
        assert_eq!(sent.len(), 38 + count * 4);
        let levels: Vec<i32> = sent[38..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| i32::from_be_bytes(*b))
            .filter(|n| (-32..=-23).contains(n) || (-256..=-247).contains(n))
            .collect();
        assert_eq!(levels, expected);
    }
}
#[test]
fn hextile_rre_and_zlib_have_exact_colors() {
    let mut d = rfb::Decoder::new(4, 4).unwrap();
    d.rectangle(
        &mut Cursor::new([14, 1, 2, 3, 0, 4, 5, 6, 0, 1, 0x11, 0x11]),
        0,
        0,
        4,
        4,
        5,
    )
    .unwrap();
    assert_eq!(rgb(&d, 0, 0), [3, 2, 1, 255]);
    assert_eq!(rgb(&d, 2, 2), [6, 5, 4, 255]);
    let mut rre = 1u32.to_be_bytes().to_vec();
    rre.extend([10, 20, 30, 0, 100, 110, 120, 0]);
    for n in [1u16, 1, 2, 2] {
        rre.extend(n.to_be_bytes());
    }
    d.rectangle(&mut Cursor::new(rre), 0, 0, 4, 4, 2).unwrap();
    assert_eq!(rgb(&d, 2, 2), [120, 110, 100, 255]);
    let raw = [40, 50, 60, 0].repeat(16);
    let packed = pack(&mut Compress::new(Compression::fast(), true), &raw);
    let mut stream = (packed.len() as u32).to_be_bytes().to_vec();
    stream.extend(packed);
    d.rectangle(&mut Cursor::new(stream), 0, 0, 4, 4, 6)
        .unwrap();
    assert_eq!(rgb(&d, 3, 3), [60, 50, 40, 255]);
}

#[test]
fn wheel_respects_units_accumulates_points_and_keeps_both_axes() {
    use egui::{MouseWheelUnit as Unit, Vec2};
    let viewport = Vec2::new(1920., 1080.);
    let mut wheel = input::RemoteWheel::default();
    let line = wheel.translate((Unit::Line, Vec2::new(0., -3.)), viewport, 3., (50, 60), 1);
    assert_eq!(line.len(), 9 * 12); // Three native lines at 3×, not one forced step.
    for pair in line.as_chunks::<12>().0 {
        assert_eq!(&pair[..6], rfb::pointer(50, 60, 1 | 16));
        assert_eq!(&pair[6..], rfb::pointer(50, 60, 1));
    }
    wheel.reset();
    for _ in 0..3 {
        assert!(
            wheel
                .translate((Unit::Point, Vec2::new(0., 5.)), viewport, 1., (0, 0), 0)
                .is_empty()
        );
    }
    assert_eq!(
        wheel
            .translate((Unit::Point, Vec2::new(0., 5.)), viewport, 1., (0, 0), 0)
            .len(),
        12
    );
    wheel.reset();
    let both = wheel.translate((Unit::Line, Vec2::new(1., -1.)), viewport, 1., (2, 3), 4);
    assert_eq!(both.len(), 24);
    assert_eq!(both[1], 16 | 4);
    assert_eq!(both[13], 32 | 4);
    assert!(
        wheel
            .translate((Unit::Point, Vec2::ZERO), viewport, 1., (0, 0), 0)
            .is_empty()
    );
    assert!(
        wheel
            .translate(
                (Unit::Point, Vec2::new(f32::NAN, 2.)),
                viewport,
                1.,
                (0, 0),
                0
            )
            .is_empty()
    );
    assert_eq!(
        wheel
            .translate((Unit::Page, Vec2::new(0., 1.)), viewport, 1., (0, 0), 0)
            .len(),
        54 * 12
    );
    wheel.reset();
    assert_eq!(
        wheel
            .translate((Unit::Line, Vec2::new(0., 3.)), viewport, 20., (0, 0), 0)
            .len(),
        60 * 12
    );
    wheel.reset();
    assert_eq!(
        wheel
            .translate((Unit::Line, Vec2::new(0., 1e9)), viewport, 8., (0, 0), 0)
            .len(),
        64 * 12
    );
}

#[test]
fn session_scaling_fills_the_viewport_without_cropping_and_keeps_alternatives() {
    let native = Vec2::new(1920., 1080.);
    for available in [
        Vec2::new(1920., 1038.),
        Vec2::new(580., 478.),
        Vec2::new(2560., 1398.),
    ] {
        assert_eq!(DisplayMode::Fill.size(native, available), available);
        let fit = DisplayMode::Fit.size(native, available);
        assert!(fit.x <= available.x + 0.01 && fit.y <= available.y + 0.01);
        assert!((fit.x / fit.y - 16. / 9.).abs() < 0.001);
        assert_eq!(DisplayMode::Actual.size(native, available), native);
    }
}

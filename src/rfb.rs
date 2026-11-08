//! Original RFB client. Network/decode work never runs in the UI thread.
use des::{
    Des,
    cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray},
};
use eframe::egui;
use flate2::{Decompress, FlushDecompress};
use std::{
    collections::VecDeque,
    io::{self, BufReader, Cursor, Read, Write},
    net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Duration,
};

pub const DEFAULT_PORT: u16 = 5900;
pub const LISTEN_PORT: u16 = 5500;
pub const JPEG_DEFAULT: i32 = 6;
// Recovered from tvnviewer.exe: 0x14010e490, 0x1401bded0, 0x1401c1060.
pub const COMPRESSION_AUTO: i32 = -1;
const JPEG_ENCODING_BASE: i32 = -32;
const COMPRESSION_ENCODING_BASE: i32 = -256;

fn level_encoding(base: i32, level: i32) -> Option<i32> {
    (0..=9).contains(&level).then_some(base + level)
}

pub(crate) fn jpeg_encoding(level: i32) -> Option<i32> {
    level_encoding(JPEG_ENCODING_BASE, level)
}

pub(crate) fn compression_encoding(level: i32) -> Option<i32> {
    level_encoding(COMPRESSION_ENCODING_BASE, level)
}

pub const MAX_PIXELS: usize = 33_554_432;
const MAX_PAYLOAD: usize = 128 * 1024 * 1024;
type Result<T> = std::result::Result<T, String>;
fn err(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
fn u8r(r: &mut impl Read) -> io::Result<u8> {
    let mut a = [0];
    r.read_exact(&mut a)?;
    Ok(a[0])
}
fn u16r(r: &mut impl Read) -> io::Result<u16> {
    let mut a = [0; 2];
    r.read_exact(&mut a)?;
    Ok(u16::from_be_bytes(a))
}
fn u32r(r: &mut impl Read) -> io::Result<u32> {
    let mut a = [0; 4];
    r.read_exact(&mut a)?;
    Ok(u32::from_be_bytes(a))
}
fn bytes(r: &mut impl Read, n: usize) -> io::Result<Vec<u8>> {
    if n > MAX_PAYLOAD {
        return Err(err("Messaggio troppo grande"));
    }
    let mut a = vec![0; n];
    r.read_exact(&mut a)?;
    Ok(a)
}
fn text(r: &mut impl Read) -> io::Result<String> {
    let n = u32r(r)? as usize;
    if n > 1_048_576 {
        return Err(err("Testo troppo lungo"));
    }
    Ok(String::from_utf8_lossy(&bytes(r, n)?).into_owned())
}
fn compact(r: &mut impl Read) -> io::Result<usize> {
    let a = u8r(r)?;
    let mut n = (a & 127) as usize;
    if a & 128 != 0 {
        let b = u8r(r)?;
        n |= ((b & 127) as usize) << 7;
        if b & 128 != 0 {
            n |= (u8r(r)? as usize) << 14;
        }
    }
    Ok(n)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum Quality {
    #[default]
    Sharp,
    Balanced,
    Fluid,
}
impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sharp => "Nitida",
            Self::Balanced => "Bilanciata",
            Self::Fluid => "Fluida",
        }
    }
    pub fn encodings(self) -> Vec<i32> {
        let mut a = vec![7, 16, 1, 5, 6, 2, 0, -239, -223, -224, -313];
        // 0x14013dce0 installs only valid levels; auto/lossless adds no level.
        let (jpeg, compression) = match self {
            Self::Sharp => (None, COMPRESSION_AUTO),
            Self::Balanced => (Some(JPEG_DEFAULT), COMPRESSION_AUTO),
            Self::Fluid => (Some(9), 1),
        };
        if let Some(encoding) = jpeg.and_then(jpeg_encoding) {
            a.push(encoding);
        }
        if let Some(encoding) = compression_encoding(compression) {
            a.push(encoding);
        }
        a
    }
}
#[derive(Clone)]
pub struct Options {
    pub host: String,
    pub port: u16,
    pub password: String,
    pub quality: Quality,
    pub shared: bool,
    pub listen: bool,
    pub listen_port: u16,
    pub clipboard: bool,
}

#[derive(Default)]
pub struct Shared {
    pub status: String,
    pub connected: bool,
    pub failed: bool,
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub patches: VecDeque<Patch>,
    pub frames: u64,
    pub received: u64,
    pub encoding: String,
    pub decode_ms: f64,
    pub coalesced_updates: u64,
    pub continuous: bool,
    pub clipboard: Option<String>,
    pub cursor: Option<RemoteCursor>,
    pub bell: u64,
}
pub struct RemoteCursor {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub hot_x: usize,
    pub hot_y: usize,
}
pub struct Patch {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<egui::Color32>,
}
impl Patch {
    pub fn into_image(self) -> egui::ColorImage {
        egui::ColorImage::new([self.w, self.h], self.pixels)
    }
}
pub enum Command {
    Bytes(Vec<u8>),
    Refresh(u16, u16),
    Stop,
}
pub struct Session {
    pub state: Arc<Mutex<Shared>>,
    pub tx: Sender<Command>,
    cancel: Arc<AtomicBool>,
    socket: Arc<Mutex<Option<TcpStream>>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}
impl Session {
    pub fn start(options: Options, ctx: egui::Context) -> Self {
        let state = Arc::new(Mutex::new(Shared {
            status: if options.listen {
                "In attesa del computer…".into()
            } else {
                "Connessione in corso…".into()
            },
            ..Default::default()
        }));
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let socket = Arc::new(Mutex::new(None));
        let (st, ca, so, send) = (state.clone(), cancel.clone(), socket.clone(), tx.clone());
        thread::spawn(move || {
            let result = (|| -> Result<()> {
                let stream = if options.listen {
                    let listener =
                        TcpListener::bind(("0.0.0.0", options.listen_port)).map_err(|e| {
                            format!(
                                "Impossibile ascoltare sulla porta {}: {e}",
                                options.listen_port
                            )
                        })?;
                    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
                    loop {
                        if ca.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        match listener.accept() {
                            Ok((s, _)) => break s,
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(40))
                            }
                            Err(e) => return Err(e.to_string()),
                        }
                    }
                } else {
                    let addresses = (options.host.as_str(), options.port)
                        .to_socket_addrs()
                        .map_err(|_| {
                            "Indirizzo non trovato. Controlla il nome o l’IP del computer."
                                .to_string()
                        })?;
                    let mut connected = None;
                    let mut last = String::new();
                    for a in addresses {
                        if ca.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        match TcpStream::connect_timeout(&a, Duration::from_secs(6)) {
                            Ok(s) => {
                                connected = Some(s);
                                break;
                            }
                            Err(e) => last = e.to_string(),
                        }
                    }
                    connected.ok_or_else(||format!("Il computer non risponde. Controlla indirizzo, server VNC e rete. {last}"))?
                };
                if ca.load(Ordering::Relaxed) {
                    return Ok(());
                }
                stream.set_nodelay(true).map_err(|e| e.to_string())?;
                stream
                    .set_read_timeout(Some(Duration::from_secs(15)))
                    .map_err(|e| e.to_string())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(6)))
                    .map_err(|e| e.to_string())?;
                *so.lock().unwrap() = Some(stream.try_clone().map_err(|e| e.to_string())?);
                if ca.load(Ordering::Relaxed) {
                    let _ = stream.shutdown(Shutdown::Both);
                    return Ok(());
                }
                let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
                let mut reader = BufReader::with_capacity(256 * 1024, stream);
                let (w, h, name) =
                    handshake(&mut reader, &mut writer, &options).map_err(|e| e.to_string())?;
                reader
                    .get_ref()
                    .set_read_timeout(None)
                    .map_err(|e| e.to_string())?;
                {
                    let mut s = st.lock().unwrap();
                    s.connected = true;
                    s.name = name;
                    s.width = w;
                    s.height = h;
                    s.status = "Connesso".into();
                }
                ctx.request_repaint();

                thread::spawn(move || {
                    while let Ok(cmd) = rx.recv() {
                        let data = match cmd {
                            Command::Bytes(a) => a,
                            Command::Refresh(w, h) => request(w, h, false),
                            Command::Stop => break,
                        };
                        if writer.write_all(&data).is_err() {
                            break;
                        }
                    }
                    let _ = writer.shutdown(Shutdown::Both);
                });
                let mut decoder = Decoder::new(w, h)?;
                send.send(Command::Bytes(request(w as u16, h as u16, false)))
                    .map_err(|e| e.to_string())?;
                while !ca.load(Ordering::Relaxed) {
                    decoder
                        .message(&mut reader, &st, &send, &ctx, options.clipboard)
                        .map_err(|e| format!("Connessione interrotta: {e}"))?;
                }
                Ok(())
            })();
            {
                let mut s = st.lock().unwrap();
                s.connected = false;
                if !ca.load(Ordering::Relaxed)
                    && let Err(e) = result
                {
                    s.status = e;
                    s.failed = true;
                }
            }
            ca.store(true, Ordering::Relaxed);
            let _ = send.send(Command::Stop);
            if let Some(socket) = so.lock().unwrap().take() {
                let _ = socket.shutdown(Shutdown::Both);
            }
            ctx.request_repaint();
        });
        Self {
            state,
            tx,
            cancel,
            socket,
        }
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Command::Stop);
        if let Some(s) = self.socket.lock().unwrap().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
    pub fn send(&self, data: Vec<u8>) {
        let _ = self.tx.send(Command::Bytes(data));
    }
}

pub fn parse_address(input: &str, default: u16) -> Result<(String, u16)> {
    let a = input.trim().strip_prefix("vnc://").unwrap_or(input.trim());
    if a.is_empty() {
        return Err("Inserisci l’indirizzo del computer.".into());
    }
    let parse_port = |s: &str| -> Result<u16> {
        s.parse::<u16>()
            .ok()
            .filter(|n| *n != 0)
            .ok_or_else(|| "La porta deve essere un numero tra 1 e 65535.".into())
    };
    let (host, port) = if a.starts_with('[') {
        let end = a.find(']').ok_or("Indirizzo IPv6 incompleto.")?;
        let tail = &a[end + 1..];
        let p = if tail.is_empty() {
            default
        } else {
            parse_port(
                tail.strip_prefix("::")
                    .or_else(|| tail.strip_prefix(':'))
                    .ok_or("Porta non valida.")?,
            )?
        };
        (a[1..end].to_string(), p)
    } else if a.parse::<std::net::Ipv6Addr>().is_ok() {
        (a.into(), default)
    } else if let Some((h, p)) = a.rsplit_once("::") {
        (h.into(), parse_port(p)?)
    } else if a.matches(':').count() == 1 {
        let (h, p) = a.split_once(':').unwrap();
        let n = if p == "0" { 0 } else { parse_port(p)? };
        // 0x1401935b0 adds the stored base port, not a fixed literal.
        // Our editable default is that base; reject overflow of the TCP port.
        let port = if n < 100 {
            default
                .checked_add(n)
                .filter(|port| *port != 0)
                .ok_or("La porta risultante deve essere tra 1 e 65535.")?
        } else {
            n
        };
        (h.into(), port)
    } else {
        (a.into(), default)
    };
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\\')
    {
        return Err("Usa un nome o un indirizzo IP, ad esempio 192.168.1.10.".into());
    }
    Ok((host, port))
}

pub fn vnc_response(password: &str, mut challenge: [u8; 16]) -> [u8; 16] {
    let mut key = [0; 8];
    for (k, b) in key.iter_mut().zip(password.as_bytes()) {
        *k = b.reverse_bits();
    }
    let cipher = Des::new(GenericArray::from_slice(&key));
    for part in challenge.as_chunks_mut::<8>().0 {
        cipher.encrypt_block(GenericArray::from_mut_slice(part));
    }
    key.fill(0);
    challenge
}

pub(crate) fn handshake(
    r: &mut impl Read,
    w: &mut impl Write,
    o: &Options,
) -> io::Result<(usize, usize, String)> {
    let banner = bytes(r, 12)?;
    // Exact syntax and future-version fallback recovered from 0x14019db20.
    if &banner[..4] != b"RFB "
        || !banner[4..7].iter().all(u8::is_ascii_digit)
        || banner[7] != b'.'
        || !banner[8..11].iter().all(u8::is_ascii_digit)
        || banner[11] != b'\n'
    {
        return Err(err("Questo computer non parla il protocollo VNC."));
    }
    let major = std::str::from_utf8(&banner[4..7])
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = std::str::from_utf8(&banner[8..11])
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    if major < 3 || (major == 3 && minor < 3) {
        return Err(err("Versione VNC non supportata"));
    }
    let v = if major > 3 || minor >= 8 {
        8
    } else if minor >= 7 {
        7
    } else {
        3
    };
    w.write_all(format!("RFB 003.{v:03}\n").as_bytes())?;
    let mut security = if v == 3 {
        u32r(r)?
    } else {
        let n = u8r(r)? as usize;
        if n == 0 {
            return Err(err(&text(r)?));
        }
        let types = bytes(r, n)?;
        let selected = if types.contains(&2) {
            2
        } else if types.contains(&1) {
            1
        } else if types.contains(&16) {
            16
        } else {
            return Err(err(
                "Il server richiede un tipo di protezione non supportato. Usa il servizio VNC classico tramite rete fidata o VPN.",
            ));
        };
        w.write_all(&[selected])?;
        selected as u32
    };
    if security == 0 {
        return Err(err(&text(r)?));
    }
    let tight = security == 16;
    if tight {
        let n = u32r(r)? as usize;
        if n > 256 {
            return Err(err("Troppe opzioni tunnel"));
        }
        let caps = bytes(r, n * 16)?;
        if n > 0 {
            if !caps
                .as_chunks::<16>()
                .0
                .iter()
                .any(|c| c[..4] == [0, 0, 0, 0])
            {
                return Err(err("Tunnel Tight non supportato"));
            }
            w.write_all(&0u32.to_be_bytes())?;
        }
        let n = u32r(r)? as usize;
        if n > 256 {
            return Err(err("Troppe opzioni di autenticazione"));
        }
        let caps = bytes(r, n * 16)?;
        if n == 0 {
            security = 1
        } else {
            security = if caps
                .as_chunks::<16>()
                .0
                .iter()
                .any(|c| c[..4] == 2u32.to_be_bytes())
            {
                2
            } else if caps
                .as_chunks::<16>()
                .0
                .iter()
                .any(|c| c[..4] == 1u32.to_be_bytes())
            {
                1
            } else {
                return Err(err("Autenticazione Tight non supportata"));
            };
            w.write_all(&security.to_be_bytes())?;
        }
    }
    if security == 2 {
        let challenge: [u8; 16] = bytes(r, 16)?.try_into().unwrap();
        w.write_all(&vnc_response(&o.password, challenge))?;
    } else if security != 1 {
        return Err(err("Tipo di autenticazione VNC non supportato"));
    }
    if security == 2 || v >= 8 {
        let status = u32r(r)?;
        if status != 0 {
            let reason = if v >= 8 {
                text(r)?
            } else {
                "Password non corretta o accesso rifiutato.".into()
            };
            return Err(err(&format!("Accesso rifiutato. {reason}")));
        }
    }
    w.write_all(&[u8::from(o.shared)])?;
    let width = u16r(r)? as usize;
    let height = u16r(r)? as usize;
    let _format = bytes(r, 16)?;
    let name = text(r)?;
    if width == 0 || height == 0 || width * height > MAX_PIXELS {
        return Err(err("Dimensioni dello schermo non supportate"));
    }
    if tight {
        let server = u16r(r)? as usize;
        let client = u16r(r)? as usize;
        let enc = u16r(r)? as usize;
        bytes(r, 2)?;
        bytes(r, (server + client + enc) * 16)?;
    }
    // Little-endian, 32 bits per pixel / 24-bit RGB, shifts R=16 G=8 B=0.
    w.write_all(&[
        0, 0, 0, 0, 32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0,
    ])?;
    w.write_all(&set_encodings(o.quality))?;
    w.flush()?;
    Ok((width, height, name))
}
pub fn set_encodings(quality: Quality) -> Vec<u8> {
    let enc = quality.encodings();
    let mut msg = vec![2, 0];
    msg.extend_from_slice(&(enc.len() as u16).to_be_bytes());
    for e in enc {
        msg.extend_from_slice(&e.to_be_bytes());
    }
    msg
}
pub fn request(w: u16, h: u16, incremental: bool) -> Vec<u8> {
    let mut a = vec![3, u8::from(incremental), 0, 0, 0, 0];
    a.extend(w.to_be_bytes());
    a.extend(h.to_be_bytes());
    a
}
fn continuous_updates(w: u16, h: u16) -> Vec<u8> {
    let mut message = vec![150, 1, 0, 0, 0, 0];
    message.extend(w.to_be_bytes());
    message.extend(h.to_be_bytes());
    message
}
pub fn key(symbol: u32, down: bool) -> Vec<u8> {
    let mut a = vec![4, u8::from(down), 0, 0];
    a.extend(symbol.to_be_bytes());
    a
}
pub fn pointer(x: u16, y: u16, mask: u8) -> Vec<u8> {
    let mut a = vec![5, mask];
    a.extend(x.to_be_bytes());
    a.extend(y.to_be_bytes());
    a
}
pub fn cut_text(value: &str) -> Vec<u8> {
    let data: Vec<u8> = value
        .chars()
        .take(1_048_576)
        .map(|c| if c as u32 <= 255 { c as u8 } else { b'?' })
        .collect();
    let mut a = vec![6, 0, 0, 0];
    a.extend((data.len() as u32).to_be_bytes());
    a.extend(data);
    a
}

pub struct Decoder {
    pub w: usize,
    pub h: usize,
    pub pixels: Vec<u8>,
    tight: [Decompress; 4],
    zlib: Decompress,
    zrle: Decompress,
    pub frames: u64,
    pub received: u64,
    continuous: bool,
    continuous_supported: bool,
}
fn inflate(d: &mut Decompress, input: &[u8], max: usize) -> io::Result<Vec<u8>> {
    let mut result = Vec::with_capacity(max.min(65536));
    let mut used = 0;
    loop {
        let mut block = [0u8; 32768];
        let before_in = d.total_in();
        let before_out = d.total_out();
        let status = d
            .decompress(&input[used..], &mut block, FlushDecompress::Sync)
            .map_err(|_| err("Dati compressi non validi"))?;
        let n = (d.total_out() - before_out) as usize;
        used += (d.total_in() - before_in) as usize;
        if result.len() + n > max {
            return Err(err("Dati decompressi troppo grandi"));
        }
        result.extend_from_slice(&block[..n]);
        if used == input.len() && n < block.len() {
            break;
        }
        if status == flate2::Status::StreamEnd {
            break;
        }
        if d.total_in() == before_in && n == 0 {
            return Err(err("Flusso compresso incompleto"));
        }
    }
    Ok(result)
}
impl Decoder {
    pub fn new(w: usize, h: usize) -> Result<Self> {
        if w == 0 || h == 0 || w * h > MAX_PIXELS {
            return Err("Schermo troppo grande".into());
        }
        let mut pixels = vec![0; w * h * 4];
        for p in pixels.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
        Ok(Self {
            w,
            h,
            pixels,
            tight: std::array::from_fn(|_| Decompress::new(true)),
            zlib: Decompress::new(true),
            zrle: Decompress::new(true),
            frames: 0,
            received: 0,
            continuous: false,
            continuous_supported: false,
        })
    }
    fn bounds(&self, x: usize, y: usize, w: usize, h: usize) -> io::Result<()> {
        if w == 0 || h == 0 || x + w > self.w || y + h > self.h {
            return Err(err("Rettangolo fuori dallo schermo"));
        }
        Ok(())
    }
    fn put(&mut self, x: usize, y: usize, p: [u8; 4]) {
        let n = (y * self.w + x) * 4;
        self.pixels[n..n + 4].copy_from_slice(&p);
    }
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, p: [u8; 4]) -> io::Result<()> {
        self.bounds(x, y, w, h)?;
        for row in self.pixels.chunks_exact_mut(self.w * 4).skip(y).take(h) {
            row[x * 4..(x + w) * 4].as_chunks_mut::<4>().0.fill(p);
        }
        Ok(())
    }
    fn pixel(r: &mut impl Read) -> io::Result<[u8; 4]> {
        let mut p = [0; 4];
        r.read_exact(&mut p)?;
        Ok([p[2], p[1], p[0], 255])
    }
    fn cpixel(r: &mut impl Read) -> io::Result<[u8; 4]> {
        let mut p = [0; 3];
        r.read_exact(&mut p)?;
        Ok([p[2], p[1], p[0], 255])
    }
    fn patch(&self, x: usize, y: usize, w: usize, h: usize) -> Patch {
        let mut pixels = Vec::with_capacity(w * h);
        for yy in y..y + h {
            pixels.extend(
                self.pixels[(yy * self.w + x) * 4..(yy * self.w + x + w) * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2])),
            );
        }
        Patch { x, y, w, h, pixels }
    }
    fn raw(&mut self, r: &mut impl Read, x: usize, y: usize, w: usize, h: usize) -> io::Result<()> {
        for row in self.pixels.chunks_exact_mut(self.w * 4).skip(y).take(h) {
            let target = &mut row[x * 4..(x + w) * 4];
            r.read_exact(target)?;
            for p in target.as_chunks_mut::<4>().0 {
                p.swap(0, 2);
                p[3] = 255;
            }
        }
        Ok(())
    }
    pub fn rectangle(
        &mut self,
        r: &mut impl Read,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        enc: i32,
    ) -> io::Result<()> {
        self.bounds(x, y, w, h)?;
        match enc {
            0 => self.raw(r, x, y, w, h)?,
            1 => {
                let sx = u16r(r)? as usize;
                let sy = u16r(r)? as usize;
                self.bounds(sx, sy, w, h)?;
                if y > sy {
                    for row in (0..h).rev() {
                        let from = ((sy + row) * self.w + sx) * 4;
                        let to = ((y + row) * self.w + x) * 4;
                        self.pixels.copy_within(from..from + w * 4, to);
                    }
                } else {
                    for row in 0..h {
                        let from = ((sy + row) * self.w + sx) * 4;
                        let to = ((y + row) * self.w + x) * 4;
                        self.pixels.copy_within(from..from + w * 4, to);
                    }
                }
            }
            2 => {
                let count = u32r(r)? as usize;
                if count > MAX_PIXELS {
                    return Err(err("Troppi rettangoli RRE"));
                }
                let bg = Self::pixel(r)?;
                self.fill(x, y, w, h, bg)?;
                for _ in 0..count {
                    let p = Self::pixel(r)?;
                    let xx = u16r(r)? as usize;
                    let yy = u16r(r)? as usize;
                    let ww = u16r(r)? as usize;
                    let hh = u16r(r)? as usize;
                    if xx + ww > w || yy + hh > h {
                        return Err(err("RRE non valido"));
                    }
                    self.fill(x + xx, y + yy, ww, hh, p)?;
                }
            }
            5 => {
                let (mut bg, mut fg) = ([0, 0, 0, 255], [0, 0, 0, 255]);
                for ty in (0..h).step_by(16) {
                    for tx in (0..w).step_by(16) {
                        let tw = (w - tx).min(16);
                        let th = (h - ty).min(16);
                        let flags = u8r(r)?;
                        if flags & 1 != 0 {
                            self.raw(r, x + tx, y + ty, tw, th)?;
                            continue;
                        }
                        if flags & 2 != 0 {
                            bg = Self::pixel(r)?;
                        }
                        if flags & 4 != 0 {
                            fg = Self::pixel(r)?;
                        }
                        self.fill(x + tx, y + ty, tw, th, bg)?;
                        if flags & 8 != 0 {
                            for _ in 0..u8r(r)? {
                                let p = if flags & 16 != 0 { Self::pixel(r)? } else { fg };
                                let xy = u8r(r)?;
                                let wh = u8r(r)?;
                                let (xx, yy, ww, hh) = (
                                    (xy >> 4) as usize,
                                    (xy & 15) as usize,
                                    (wh >> 4) as usize + 1,
                                    (wh & 15) as usize + 1,
                                );
                                if xx + ww > tw || yy + hh > th {
                                    return Err(err("Hextile non valido"));
                                }
                                self.fill(x + tx + xx, y + ty + yy, ww, hh, p)?;
                            }
                        }
                    }
                }
            }
            6 => {
                let n = u32r(r)? as usize;
                let compressed = bytes(r, n)?;
                let raw = inflate(&mut self.zlib, &compressed, w * h * 4)?;
                if raw.len() != w * h * 4 {
                    return Err(err("zlib incompleto"));
                }
                self.raw(&mut Cursor::new(raw), x, y, w, h)?;
            }
            7 => self.decode_tight(r, x, y, w, h)?,
            16 => self.decode_zrle(r, x, y, w, h)?,
            _ => return Err(err(&format!("Codifica VNC non supportata: {enc}"))),
        }
        Ok(())
    }
    fn decode_tight(
        &mut self,
        r: &mut impl Read,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> io::Result<()> {
        let ctl = u8r(r)?;
        for (i, stream) in self.tight.iter_mut().enumerate() {
            if ctl & (1 << i) != 0 {
                *stream = Decompress::new(true);
            }
        }
        let mode = ctl >> 4;
        if mode == 8 {
            let p = bytes(r, 3)?;
            return self.fill(x, y, w, h, [p[0], p[1], p[2], 255]);
        }
        if mode == 9 {
            let n = compact(r)?;
            let compressed = bytes(r, n)?;
            let mut jpeg = jpeg_decoder::Decoder::new(Cursor::new(compressed));
            jpeg.set_max_decoding_buffer_size(MAX_PAYLOAD);
            let rgb = jpeg.decode().map_err(|_| err("Immagine JPEG non valida"))?;
            let info = jpeg.info().ok_or(err("JPEG vuoto"))?;
            if info.width as usize != w || info.height as usize != h {
                return Err(err("Dimensioni JPEG errate"));
            }
            let stride = info.pixel_format.pixel_bytes();
            if stride != 1 && stride != 3 {
                return Err(err("Formato JPEG non supportato"));
            }
            for yy in 0..h {
                for xx in 0..w {
                    let i = (yy * w + xx) * stride;
                    self.put(
                        x + xx,
                        y + yy,
                        if stride == 1 {
                            [rgb[i], rgb[i], rgb[i], 255]
                        } else {
                            [rgb[i], rgb[i + 1], rgb[i + 2], 255]
                        },
                    );
                }
            }
            return Ok(());
        }
        if mode > 7 {
            return Err(err("Compressione Tight non valida"));
        }
        let filter = if mode & 4 != 0 { u8r(r)? } else { 0 };
        let mut palette = Vec::new();
        if filter == 1 {
            let count = u8r(r)? as usize + 1;
            for _ in 0..count {
                let p = bytes(r, 3)?;
                palette.push([p[0], p[1], p[2], 255]);
            }
        } else if filter > 2 {
            return Err(err("Filtro Tight non supportato"));
        }
        let expected = if filter == 1 {
            if palette.len() == 2 {
                w.div_ceil(8) * h
            } else {
                w * h
            }
        } else {
            w * h * 3
        };
        let data = if expected < 12 {
            bytes(r, expected)?
        } else {
            let n = compact(r)?;
            let packed = bytes(r, n)?;
            inflate(&mut self.tight[(mode & 3) as usize], &packed, expected)?
        };
        if data.len() != expected {
            return Err(err("Rettangolo Tight incompleto"));
        }
        let mut decoded = vec![0u8; w * h * 3];
        for yy in 0..h {
            for xx in 0..w {
                let i = (yy * w + xx) * 3;
                let p = if filter == 1 {
                    let idx = if palette.len() == 2 {
                        ((data[yy * w.div_ceil(8) + xx / 8] >> (7 - xx % 8)) & 1) as usize
                    } else {
                        data[yy * w + xx] as usize
                    };
                    *palette.get(idx).ok_or(err("Indice palette non valido"))?
                } else if filter == 2 {
                    let mut p = [0, 0, 0, 255];
                    for c in 0..3 {
                        let left = if xx > 0 { decoded[i - 3 + c] as i32 } else { 0 };
                        let up = if yy > 0 {
                            decoded[i - w * 3 + c] as i32
                        } else {
                            0
                        };
                        let ul = if xx > 0 && yy > 0 {
                            decoded[i - w * 3 - 3 + c] as i32
                        } else {
                            0
                        };
                        p[c] = data[i + c].wrapping_add((left + up - ul).clamp(0, 255) as u8);
                        decoded[i + c] = p[c];
                    }
                    p
                } else {
                    [data[i], data[i + 1], data[i + 2], 255]
                };
                self.put(x + xx, y + yy, p);
            }
        }
        Ok(())
    }
    fn decode_zrle(
        &mut self,
        r: &mut impl Read,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
    ) -> io::Result<()> {
        let n = u32r(r)? as usize;
        let packed = bytes(r, n)?;
        let data = inflate(
            &mut self.zrle,
            &packed,
            (w * h * 5 + ((w.div_ceil(64) * h.div_ceil(64)) * 769)).min(MAX_PAYLOAD),
        )?;
        let mut r = Cursor::new(data);
        for ty in (0..h).step_by(64) {
            for tx in (0..w).step_by(64) {
                let (tw, th) = ((w - tx).min(64), (h - ty).min(64));
                let mode = u8r(&mut r)?;
                let count = (mode & 127) as usize;
                let mut palette = Vec::new();
                for _ in 0..count {
                    palette.push(Self::cpixel(&mut r)?);
                }
                if mode == 0 {
                    for yy in 0..th {
                        for xx in 0..tw {
                            let p = Self::cpixel(&mut r)?;
                            self.put(x + tx + xx, y + ty + yy, p);
                        }
                    }
                } else if mode == 1 {
                    self.fill(x + tx, y + ty, tw, th, palette[0])?;
                } else if mode <= 16 {
                    let bits = if count <= 2 {
                        1
                    } else if count <= 4 {
                        2
                    } else {
                        4
                    };
                    for yy in 0..th {
                        let mut value = 0;
                        let mut remaining = 0;
                        for xx in 0..tw {
                            if remaining == 0 {
                                value = u8r(&mut r)?;
                                remaining = 8;
                            }
                            remaining -= bits;
                            let idx = ((value >> remaining) & ((1 << bits) - 1)) as usize;
                            self.put(
                                x + tx + xx,
                                y + ty + yy,
                                *palette.get(idx).ok_or(err("Palette ZRLE non valida"))?,
                            );
                        }
                    }
                } else if mode == 128 || mode >= 130 {
                    let mut index = 0;
                    while index < tw * th {
                        let mut length = 1;
                        let p = if mode == 128 {
                            Self::cpixel(&mut r)?
                        } else {
                            let idx = u8r(&mut r)?;
                            let p = *palette
                                .get((idx & 127) as usize)
                                .ok_or(err("Palette ZRLE non valida"))?;
                            if idx & 128 != 0 {
                                length_marker(&mut r, &mut length)?;
                            }
                            p
                        };
                        if mode == 128 {
                            length_marker(&mut r, &mut length)?;
                        }
                        if index + length > tw * th {
                            return Err(err("Run ZRLE fuori dai limiti"));
                        }
                        for k in index..index + length {
                            self.put(x + tx + k % tw, y + ty + k / tw, p);
                        }
                        index += length;
                    }
                } else {
                    return Err(err("Sottocodifica ZRLE non valida"));
                }
            }
        }
        Ok(())
    }
    fn message(
        &mut self,
        r: &mut impl Read,
        shared: &Arc<Mutex<Shared>>,
        tx: &Sender<Command>,
        ctx: &egui::Context,
        clipboard: bool,
    ) -> io::Result<()> {
        match u8r(r)? {
            0 => {
                bytes(r, 1)?;
                let count = u16r(r)?;
                // One request in flight: overlap server work/TCP with this frame's decode.
                if !self.continuous {
                    let _ = tx.send(Command::Bytes(request(self.w as u16, self.h as u16, true)));
                }
                let decode_started = std::time::Instant::now();
                let mut dirty: Option<(usize, usize, usize, usize)> = None;
                let mut encoding = "";
                for _ in 0..count {
                    let x = u16r(r)? as usize;
                    let y = u16r(r)? as usize;
                    let w = u16r(r)? as usize;
                    let h = u16r(r)? as usize;
                    let enc = u32r(r)? as i32;
                    match enc {
                        -224 => break,
                        -223 => {
                            if w == 0 || h == 0 || w * h > MAX_PIXELS {
                                return Err(err("Dimensioni dello schermo non valide"));
                            }
                            self.w = w;
                            self.h = h;
                            self.pixels = vec![0; w * h * 4];
                            for p in self.pixels.as_chunks_mut::<4>().0 {
                                p[3] = 255;
                            }
                            dirty = Some((0, 0, w, h));
                            if self.continuous {
                                let _ =
                                    tx.send(Command::Bytes(continuous_updates(w as u16, h as u16)));
                            }
                            let _ = tx.send(Command::Refresh(w as u16, h as u16));
                        }
                        -239 => {
                            if w * h > 1_048_576 {
                                return Err(err("Cursore troppo grande"));
                            }
                            let mut p = bytes(r, w * h * 4)?;
                            let mask = bytes(r, w.div_ceil(8) * h)?;
                            for yy in 0..h {
                                for xx in 0..w {
                                    let i = (yy * w + xx) * 4;
                                    p.swap(i, i + 2);
                                    p[i + 3] = if mask[yy * w.div_ceil(8) + xx / 8]
                                        & (128 >> (xx % 8))
                                        != 0
                                    {
                                        255
                                    } else {
                                        0
                                    };
                                }
                            }
                            shared.lock().unwrap().cursor = Some(RemoteCursor {
                                pixels: p,
                                width: w,
                                height: h,
                                hot_x: x,
                                hot_y: y,
                            });
                        }
                        _ => {
                            self.rectangle(r, x, y, w, h, enc)?;
                            encoding = match enc {
                                7 => "Tight",
                                16 => "ZRLE",
                                5 => "Hextile",
                                6 => "zlib",
                                1 => "CopyRect",
                                2 => "RRE",
                                _ => "Raw",
                            };
                            dirty = Some(match dirty {
                                Some((l, t, rr, b)) => {
                                    (l.min(x), t.min(y), rr.max(x + w), b.max(y + h))
                                }
                                None => (x, y, x + w, y + h),
                            });
                            self.received += (w * h * 4) as u64;
                        }
                    }
                }
                if let Some((mut x, mut y, mut right, mut bottom)) = dirty {
                    {
                        let mut s = shared.lock().unwrap();
                        if s.width != self.w || s.height != self.h {
                            s.patches.clear();
                        }
                        // Keep only the newest pixels, preserving all undelivered dirty areas.
                        while let Some(pending) = s.patches.pop_front() {
                            x = x.min(pending.x);
                            y = y.min(pending.y);
                            right = right.max(pending.x + pending.w);
                            bottom = bottom.max(pending.y + pending.h);
                            s.coalesced_updates += 1;
                        }
                    }
                    // Pixel preparation runs on the decoder thread, outside the shared lock.
                    let patch = self.patch(x, y, right - x, bottom - y);
                    let mut s = shared.lock().unwrap();
                    s.width = self.w;
                    s.height = self.h;
                    s.frames += 1;
                    s.received = self.received;
                    s.decode_ms = decode_started.elapsed().as_secs_f64() * 1000.;
                    if !encoding.is_empty() {
                        s.encoding = encoding.into();
                    }
                    s.patches.push_back(patch);
                    self.frames += 1;
                }
                ctx.request_repaint();
            }
            1 => {
                bytes(r, 1)?;
                u16r(r)?;
                let n = u16r(r)? as usize;
                bytes(r, n * 6)?;
            }
            2 => {
                shared.lock().unwrap().bell += 1;
                ctx.request_repaint();
            }
            3 => {
                bytes(r, 3)?;
                let n = u32r(r)? as usize;
                if n > 1_048_576 {
                    return Err(err("Appunti troppo grandi"));
                }
                let a = bytes(r, n)?;
                if clipboard {
                    shared.lock().unwrap().clipboard =
                        Some(a.iter().map(|b| char::from(*b)).collect());
                    ctx.request_repaint();
                }
            }
            150 => {
                // The server acknowledged -313. Never send extension messages before this.
                let first = !self.continuous_supported;
                self.continuous_supported = true;
                self.continuous = first;
                shared.lock().unwrap().continuous = first;
                let message = if first {
                    continuous_updates(self.w as u16, self.h as u16)
                } else {
                    request(self.w as u16, self.h as u16, true)
                };
                let _ = tx.send(Command::Bytes(message));
            }
            v => return Err(err(&format!("Messaggio server sconosciuto: {v}"))),
        }
        Ok(())
    }
}
fn length_marker(r: &mut impl Read, length: &mut usize) -> io::Result<()> {
    loop {
        let n = u8r(r)? as usize;
        *length += n;
        if *length > MAX_PIXELS {
            return Err(err("Run troppo lungo"));
        }
        if n < 255 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;

    fn fill_packet(x: u16, y: u16, w: u16, h: u16, color: [u8; 3]) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 1];
        for v in [x, y, w, h] {
            data.extend(v.to_be_bytes());
        }
        data.extend(7_i32.to_be_bytes());
        data.extend([0x80, color[0], color[1], color[2]]);
        data
    }

    #[test]
    fn pending_updates_keep_latest_pixels_and_undelivered_dirty_regions() {
        let mut decoder = Decoder::new(8, 4).unwrap();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, _rx) = mpsc::channel();
        let ctx = egui::Context::default();
        for data in [
            fill_packet(1, 1, 1, 1, [10, 20, 30]),
            fill_packet(3, 1, 1, 1, [40, 50, 60]),
            fill_packet(1, 1, 1, 1, [70, 80, 90]),
        ] {
            decoder
                .message(&mut Cursor::new(data), &shared, &tx, &ctx, false)
                .unwrap();
        }
        let state = shared.lock().unwrap();
        assert_eq!(state.patches.len(), 1);
        assert_eq!(state.coalesced_updates, 2);
        let patch = &state.patches[0];
        assert_eq!((patch.x, patch.y, patch.w, patch.h), (1, 1, 3, 1));
        assert_eq!(patch.pixels[0], egui::Color32::from_rgb(70, 80, 90));
        assert_eq!(patch.pixels[1], egui::Color32::BLACK);
        assert_eq!(patch.pixels[2], egui::Color32::from_rgb(40, 50, 60));
    }

    #[test]
    fn next_update_is_requested_before_reading_pixel_payload() {
        struct CheckedReader<'a> {
            bytes: Cursor<Vec<u8>>,
            requests: &'a mpsc::Receiver<Command>,
            checked: bool,
        }
        impl Read for CheckedReader<'_> {
            fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
                if self.bytes.position() >= 4 && !self.checked {
                    assert!(
                        matches!(self.requests.try_recv(), Ok(Command::Bytes(bytes)) if bytes == request(8, 4, true))
                    );
                    self.checked = true;
                }
                self.bytes.read(data)
            }
        }
        let mut decoder = Decoder::new(8, 4).unwrap();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel();
        let mut reader = CheckedReader {
            bytes: Cursor::new(fill_packet(0, 0, 1, 1, [10, 20, 30])),
            requests: &rx,
            checked: false,
        };
        decoder
            .message(&mut reader, &shared, &tx, &egui::Context::default(), false)
            .unwrap();
        assert!(reader.checked);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn continuous_updates_activate_only_after_server_ack_and_resize_the_region() {
        let mut decoder = Decoder::new(8, 4).unwrap();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (tx, rx) = mpsc::channel();
        let ctx = egui::Context::default();
        assert!(!decoder.continuous);
        decoder
            .message(&mut Cursor::new(vec![150]), &shared, &tx, &ctx, false)
            .unwrap();
        assert!(
            matches!(rx.try_recv(), Ok(Command::Bytes(bytes)) if bytes == continuous_updates(8, 4))
        );
        assert!(shared.lock().unwrap().continuous);
        decoder
            .message(
                &mut Cursor::new(fill_packet(0, 0, 1, 1, [1, 2, 3])),
                &shared,
                &tx,
                &ctx,
                false,
            )
            .unwrap();
        assert!(rx.try_recv().is_err());
        let mut resize = vec![0, 0, 0, 1];
        for v in [0_u16, 0, 3, 2] {
            resize.extend(v.to_be_bytes());
        }
        resize.extend((-223_i32).to_be_bytes());
        decoder
            .message(&mut Cursor::new(resize), &shared, &tx, &ctx, false)
            .unwrap();
        assert!(
            matches!(rx.try_recv(), Ok(Command::Bytes(bytes)) if bytes == continuous_updates(3, 2))
        );
        assert!(matches!(rx.try_recv(), Ok(Command::Refresh(3, 2))));
        let state = shared.lock().unwrap();
        assert_eq!((state.width, state.height), (3, 2));
        assert_eq!(state.patches[0].pixels.len(), 6);
        drop(state);
        decoder
            .message(&mut Cursor::new(vec![150]), &shared, &tx, &ctx, false)
            .unwrap();
        assert!(!decoder.continuous);
        assert!(matches!(rx.try_recv(), Ok(Command::Bytes(bytes)) if bytes == request(3, 2, true)));
    }

    #[test]
    #[ignore = "manual release CPU benchmark; excludes TCP and GPU"]
    fn benchmark_frame_pipeline() {
        for (w, h) in [(1920, 1080), (3840, 2160)] {
            let mut packet = vec![0, 0, 0, 1];
            for value in [0_u16, 0, w as u16, h as u16] {
                packet.extend(value.to_be_bytes());
            }
            packet.extend(0_i32.to_be_bytes());
            packet.extend([30, 80, 130, 0].repeat(w * h));
            let mut decoder = Decoder::new(w, h).unwrap();
            let shared = Arc::new(Mutex::new(Shared::default()));
            let (tx, rx) = mpsc::channel();
            let ctx = egui::Context::default();
            let mut samples = vec![];
            let mut prepare = vec![];
            for i in 0..95 {
                let start = std::time::Instant::now();
                decoder
                    .message(&mut Cursor::new(&packet), &shared, &tx, &ctx, false)
                    .unwrap();
                let patches = shared.lock().unwrap().patches.drain(..).collect::<Vec<_>>();
                let ui_start = std::time::Instant::now();
                for patch in patches {
                    std::hint::black_box(patch.into_image());
                }
                let ui_ms = ui_start.elapsed().as_secs_f64() * 1000.;
                for command in rx.try_iter() {
                    std::hint::black_box(command);
                }
                let ms = start.elapsed().as_secs_f64() * 1000.;
                if i >= 5 {
                    samples.push(ms);
                    prepare.push(ui_ms);
                }
            }
            samples.sort_by(f64::total_cmp);
            prepare.sort_by(f64::total_cmp);
            println!(
                "{}",
                serde_json::json!({ "size": [w,h], "samples": samples.len(), "cpu_pipeline_p50_ms": samples[45], "cpu_pipeline_p95_ms": samples[85], "ui_prepare_p95_ms": prepare[85], "scope": "Raw decode + publish + UI image preparation; RAM input; no network/GPU" })
            );
        }
    }
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;
mod home;
mod input;
mod rfb;
mod session_options;
#[cfg(test)]
mod tests;
use desktop::DisplayMode;
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use rfb::{DEFAULT_PORT, LISTEN_PORT, Quality, Session};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const INK: Color32 = Color32::from_rgb(13, 30, 65);
const MUTED: Color32 = Color32::from_rgb(82, 99, 128);
const ACCENT: Color32 = Color32::from_rgb(0, 74, 204);
const LINE: Color32 = Color32::from_rgb(216, 225, 241);
const BG: Color32 = Color32::from_rgb(246, 248, 252);
const SOFT: Color32 = Color32::from_rgb(233, 240, 253);

fn brand_image(bytes: &[u8]) -> egui::ColorImage {
    let image = image::load_from_memory(bytes)
        .expect("Deskora brand asset must be a valid PNG")
        .into_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Recent {
    host: String,
    port: u16,
}
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Preferences {
    port: u16,
    listen_port: u16,
    quality: Quality,
    view_only: bool,
    clipboard: bool,
    shared: bool,
    wheel_speed: f32,
    recent: Vec<Recent>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            listen_port: LISTEN_PORT,
            quality: Quality::Sharp,
            view_only: false,
            clipboard: true,
            shared: true,
            wheel_speed: 3.,
            recent: vec![],
        }
    }
}
/// Size of the start window: just enough for the form, the password drawer and five recents.
/// The height is then fitted to the content on the first frame.
const HOME_SIZE: [f32; 2] = [480., 440.];
const HOME_MIN_SIZE: [f32; 2] = [420., 320.];
const SESSION_SIZE: [f32; 2] = [1100., 760.];

/// `preferences.json` next to the executable keeps the portable mode working;
/// otherwise preferences live in `%APPDATA%\Deskora`, so the install folder stays clean.
fn config_path() -> PathBuf {
    let portable = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("."))
        .with_file_name("preferences.json");
    if portable.is_file() {
        return portable;
    }
    match std::env::var_os("APPDATA") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("Deskora").join("preferences.json"),
        _ => portable,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    /// Keep the window center, so a centered window stays centered.
    Center,
    /// Keep the title bar where it is and grow or shrink downward.
    Top,
}

/// Resizes the window around `anchor`, keeping it on the screen.
fn resize_window(ctx: &egui::Context, size: [f32; 2], anchor: Anchor) {
    let size = Vec2::from(size);
    let position = ctx.input(|i| {
        let outer = i.viewport().outer_rect?;
        let inner = i.viewport().inner_rect?;
        let decorations = outer.size() - inner.size();
        let mut min = outer.center() - (size + decorations) * 0.5;
        if anchor == Anchor::Top {
            min.y = outer.min.y;
        }
        if let Some(monitor) = i.viewport().monitor_size
            && outer.min.x >= 0.
            && outer.min.y >= 0.
        {
            // Single-monitor coordinates: keep the title bar reachable.
            min = min
                .max(egui::Pos2::ZERO)
                .min((monitor - size - decorations).max(Vec2::ZERO).to_pos2());
        }
        Some(min)
    });
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    if let Some(position) = position {
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
    }
}
#[derive(Default)]
struct Args {
    screenshot: Option<PathBuf>,
    screenshot_size: Option<[f32; 2]>,
    connect: Option<String>,
    port: Option<u16>,
    options: bool,
    help: bool,
    show_password: bool,
    sample_recents: bool,
    password_stdin: bool,
}
impl Args {
    fn read() -> Self {
        let mut s = Self::default();
        let mut a = std::env::args().skip(1);
        while let Some(v) = a.next() {
            match v.as_str() {
                "--screenshot" => s.screenshot = a.next().map(PathBuf::from),
                "--screenshot-size" => {
                    let width = a.next().and_then(|s| s.parse::<f32>().ok());
                    let height = a.next().and_then(|s| s.parse::<f32>().ok());
                    if let (Some(w), Some(h)) = (width, height)
                        && w.is_finite()
                        && h.is_finite()
                        && w >= HOME_MIN_SIZE[0]
                        && h >= HOME_MIN_SIZE[1]
                    {
                        s.screenshot_size = Some([w, h]);
                    }
                }
                "--connect" => s.connect = a.next(),
                "--port" => s.port = a.next().and_then(|s| s.parse().ok()),
                "--show-options" => s.options = true,
                "--show-help" => s.help = true,
                "--show-password" => s.show_password = true,
                "--sample-recents" => s.sample_recents = true,
                "--password-stdin" => s.password_stdin = true,
                _ => {}
            }
        }
        s
    }
}
struct App {
    brand_icon: egui::TextureHandle,
    host: String,
    password: String,
    prefs: Preferences,
    port_text: String,
    listen_text: String,
    options: bool,
    help: bool,
    show_password: bool,
    password_open: bool,
    /// Start-screen content height the window was last fitted to (0 = not fitted yet).
    fitted_height: f32,
    error: Option<String>,
    notice: Option<String>,
    session: Option<Session>,
    texture: Option<egui::TextureHandle>,
    texture_size: [usize; 2],
    cursor_texture: Option<egui::TextureHandle>,
    cursor_size: Vec2,
    cursor_hot: Vec2,
    display_mode: DisplayMode,
    fullscreen: bool,
    remembered: bool,
    keyboard: input::RemoteKeyboard,
    wheel: input::RemoteWheel,
    pointer_mask: u8,
    last_pointer: (u16, u16),
    remote_focus: bool,
    clipboard: Option<arboard::Clipboard>,
    last_clipboard: String,
    last_clipboard_check: Instant,
    fps_at: Instant,
    fps_frames: u64,
    fps: f64,
    bell: u64,
    args: Args,
    frames: u32,
    shot_requested: bool,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::light();
        style.visuals.panel_fill = BG;
        style.visuals.window_fill = Color32::WHITE;
        style.visuals.override_text_color = Some(INK);
        style.visuals.faint_bg_color = SOFT;
        style.visuals.extreme_bg_color = Color32::WHITE;
        style.visuals.hyperlink_color = ACCENT;
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
        style.visuals.widgets.inactive.bg_fill = Color32::WHITE;
        style.visuals.widgets.inactive.weak_bg_fill = Color32::WHITE;
        style.visuals.widgets.inactive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(126, 145, 177));
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.5_f32, INK);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.5_f32, ACCENT);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.5_f32, ACCENT);
        style.visuals.widgets.active.bg_stroke = Stroke::new(2.0_f32, ACCENT);
        style.visuals.widgets.active.fg_stroke = Stroke::new(2.0_f32, ACCENT);
        style.visuals.widgets.hovered.weak_bg_fill = SOFT;
        style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(216, 230, 255);
        style.visuals.selection.bg_fill = Color32::from_rgb(202, 220, 255);
        style.visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT);
        for v in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            v.corner_radius = CornerRadius::same(9);
        }
        style.spacing.item_spacing = Vec2::new(10., 10.);
        style.spacing.button_padding = Vec2::new(12., 8.);
        style.spacing.interact_size = Vec2::new(36., 36.);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.));
        style
            .text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(13.));
        cc.egui_ctx.set_style(style);
        if let Ok(font) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("system".into(), egui::FontData::from_owned(font).into());
            fonts
                .families
                .get_mut(&egui::FontFamily::Proportional)
                .unwrap()
                .insert(0, "system".into());
            let mut emphasis_fonts = fonts.families[&egui::FontFamily::Proportional].clone();
            if let Ok(font) = std::fs::read("C:/Windows/Fonts/seguisb.ttf") {
                fonts
                    .font_data
                    .insert("semibold".into(), egui::FontData::from_owned(font).into());
                emphasis_fonts.insert(0, "semibold".into());
            }
            fonts.families.insert(emphasis_family(), emphasis_fonts);
            cc.egui_ctx.set_fonts(fonts);
        } else {
            let mut fonts = egui::FontDefinitions::default();
            fonts.families.insert(
                emphasis_family(),
                fonts.families[&egui::FontFamily::Proportional].clone(),
            );
            cc.egui_ctx.set_fonts(fonts);
        }
        let mut prefs: Preferences = if args.screenshot.is_none() {
            std::fs::read(config_path())
                .ok()
                .and_then(|s| serde_json::from_slice(&s).ok())
                .unwrap_or_default()
        } else {
            Preferences::default()
        };
        if args.sample_recents {
            prefs.recent = ["192.168.1.11", "studio-mac.local", "10.0.0.25"]
                .into_iter()
                .map(|host| Recent {
                    host: host.into(),
                    port: DEFAULT_PORT,
                })
                .collect();
        }
        if let Some(port) = args.port {
            prefs.port = port;
        }
        let port_text = prefs.port.to_string();
        let listen_text = prefs.listen_port.to_string();
        let mut host = args.connect.clone().unwrap_or_default();
        let auto = args.connect.is_some();
        let options = args.options;
        let help = args.help;
        let mut password = String::new();
        let password_open = auto || args.password_stdin || args.show_password;
        if args.show_password && host.is_empty() {
            host = "192.168.1.10".into();
        }
        if args.password_stdin {
            use std::io::{BufRead, Read};
            let _ = std::io::stdin().lock().take(4096).read_line(&mut password);
            password = password.trim_end_matches(['\r', '\n']).to_owned();
        }
        let mut s = Self {
            brand_icon: cc.egui_ctx.load_texture(
                "deskora-icon",
                brand_image(include_bytes!("../assets/deskora-icon.png")),
                egui::TextureOptions::LINEAR,
            ),
            host,
            password,
            prefs,
            port_text,
            listen_text,
            options,
            help,
            show_password: false,
            password_open,
            fitted_height: 0.,
            error: None,
            notice: None,
            session: None,
            texture: None,
            texture_size: [0; 2],
            cursor_texture: None,
            cursor_size: Vec2::ZERO,
            cursor_hot: Vec2::ZERO,
            display_mode: DisplayMode::Fill,
            fullscreen: false,
            remembered: false,
            keyboard: input::RemoteKeyboard::default(),
            wheel: input::RemoteWheel::default(),
            pointer_mask: 0,
            last_pointer: (0, 0),
            remote_focus: false,
            clipboard: arboard::Clipboard::new().ok(),
            last_clipboard: String::new(),
            last_clipboard_check: Instant::now(),
            fps_at: Instant::now(),
            fps_frames: 0,
            fps: 0.,
            bell: 0,
            args,
            frames: 0,
            shot_requested: false,
        };
        if auto {
            s.connect(&cc.egui_ctx, false);
        }
        s
    }
    fn save_preferences(&mut self) {
        if let Ok(port) = self.port_text.parse::<u16>()
            && port > 0
        {
            self.prefs.port = port;
        }
        if let Ok(port) = self.listen_text.parse::<u16>()
            && port > 0
        {
            self.prefs.listen_port = port;
        }
        if self.args.screenshot.is_some() {
            return;
        }
        let path = config_path();
        let saved = serde_json::to_vec_pretty(&self.prefs)
            .map_err(|e| e.to_string())
            .and_then(|a| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, a).map_err(|e| e.to_string())
            });
        if saved.is_err() {
            self.notice = Some(format!(
                "Preferenze non salvate: impossibile scrivere {}.",
                path.display()
            ));
        }
    }
    fn connect(&mut self, ctx: &egui::Context, listen: bool) {
        self.error = None;
        let p = match self.port_text.parse::<u16>().ok().filter(|p| *p != 0) {
            Some(p) => p,
            None => {
                self.error = Some("La porta deve essere tra 1 e 65535.".into());
                self.options = true;
                return;
            }
        };
        let lp = match self.listen_text.parse::<u16>().ok().filter(|p| *p != 0) {
            Some(p) => p,
            None => {
                self.error = Some("Controlla la porta di ascolto.".into());
                return;
            }
        };
        let (host, port) = if listen {
            (String::new(), p)
        } else {
            match rfb::parse_address(&self.host, p) {
                Ok(v) => v,
                Err(e) => {
                    self.error = Some(e);
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new("remote_address")));
                    return;
                }
            }
        };
        self.prefs.port = p;
        self.prefs.listen_port = lp;
        self.save_preferences();
        self.texture = None;
        self.cursor_texture = None;
        self.texture_size = [0; 2];
        self.remembered = false;
        self.remote_focus = false;
        self.fps_at = Instant::now();
        self.fps_frames = 0;
        self.fps = 0.;
        self.bell = 0;
        self.session = Some(Session::start(
            rfb::Options {
                host,
                port,
                password: self.password.clone(),
                quality: self.prefs.quality,
                shared: self.prefs.shared,
                listen,
                listen_port: lp,
                clipboard: self.prefs.clipboard,
            },
            ctx.clone(),
        ));
    }
    fn disconnect(&mut self, ctx: &egui::Context) {
        self.release_input();
        if let Some(s) = self.session.take() {
            s.stop();
        }
        self.texture = None;
        self.cursor_texture = None;
        self.remote_focus = false;
        self.password.clear();
        if self.fullscreen {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("Deskora".into()));
        self.fitted_height = 0.;
        resize_window(ctx, HOME_SIZE, Anchor::Center);
    }
    fn release_input(&mut self) {
        self.wheel.reset();
        let mut releases = self.keyboard.release().concat();
        if let Some(s) = &self.session {
            releases.extend(rfb::pointer(self.last_pointer.0, self.last_pointer.1, 0));
            s.send(releases);
        }
        self.pointer_mask = 0;
    }
    fn pump(&mut self, ctx: &egui::Context) {
        let Some(session) = &self.session else {
            return;
        };
        let (connected, failed, status, name, w, h, patches, clip, cursor, frames, bell) = {
            let mut s = session.state.lock().unwrap();
            (
                s.connected,
                s.failed,
                s.status.clone(),
                s.name.clone(),
                s.width,
                s.height,
                s.patches.drain(..).collect::<Vec<_>>(),
                s.clipboard.take(),
                s.cursor.take(),
                s.frames,
                s.bell,
            )
        };
        if failed {
            self.error = Some(status);
            self.disconnect(ctx);
            return;
        }
        if connected && !self.remembered {
            self.remembered = true;
            self.password.clear();
            if let Ok((host, port)) = rfb::parse_address(&self.host, self.prefs.port) {
                self.prefs
                    .recent
                    .retain(|r| r.host != host || r.port != port);
                self.prefs.recent.insert(0, Recent { host, port });
                self.prefs.recent.truncate(5);
                self.save_preferences();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — Deskora")));
            let size = if self.args.screenshot.is_some() {
                self.args.screenshot_size.unwrap_or(SESSION_SIZE)
            } else {
                SESSION_SIZE
            };
            resize_window(ctx, size, Anchor::Center);
            if self.args.options {
                egui::Popup::open_id(ctx, egui::Id::new(session_options::POPUP_ID));
            }
        }
        if w > 0 && h > 0 && !patches.is_empty() {
            if self.texture_size != [w, h] {
                self.texture = Some(ctx.load_texture(
                    "desktop",
                    egui::ColorImage::filled([w, h], Color32::BLACK),
                    egui::TextureOptions::LINEAR,
                ));
                self.texture_size = [w, h];
            }
            if let Some(t) = &mut self.texture {
                for p in patches {
                    t.set_partial([p.x, p.y], p.into_image(), egui::TextureOptions::LINEAR);
                }
            }
        }
        if let Some(cursor) = cursor {
            self.cursor_size = Vec2::new(cursor.width as f32, cursor.height as f32);
            self.cursor_hot = Vec2::new(cursor.hot_x as f32, cursor.hot_y as f32);
            if cursor.width > 0 && cursor.height > 0 {
                self.cursor_texture = Some(ctx.load_texture(
                    "remote_cursor",
                    egui::ColorImage::from_rgba_unmultiplied(
                        [cursor.width, cursor.height],
                        &cursor.pixels,
                    ),
                    egui::TextureOptions::NEAREST,
                ));
            } else {
                self.cursor_texture = None;
            }
        }
        if let Some(value) = clip
            && self.prefs.clipboard
        {
            if let Some(c) = &mut self.clipboard {
                let _ = c.set_text(value.clone());
            }
            self.last_clipboard = value;
        }
        if connected
            && self.prefs.clipboard
            && self.remote_focus
            && self.last_clipboard_check.elapsed() > Duration::from_millis(500)
        {
            self.last_clipboard_check = Instant::now();
            if let Some(c) = &mut self.clipboard
                && let Ok(value) = c.get_text()
                && value != self.last_clipboard
                && value.len() <= 1_048_576
            {
                self.last_clipboard = value.clone();
                if let Some(s) = &self.session {
                    s.send(rfb::cut_text(&value));
                }
            }
        }
        if bell > self.bell {
            self.bell = bell;
            self.notice = Some("Il computer remoto ha inviato un avviso.".into());
        }
        if self.fps_at.elapsed().as_secs_f64() >= 1. {
            self.fps = (frames - self.fps_frames) as f64 / self.fps_at.elapsed().as_secs_f64();
            self.fps_frames = frames;
            self.fps_at = Instant::now();
        }
    }
    fn remote_input(&mut self, ctx: &egui::Context, response: &egui::Response, w: usize, h: usize) {
        let Some(s) = &self.session else {
            return;
        };
        let events = ctx.input(|i| i.events.clone());
        let hovered = response.hovered();
        if self.remote_focus {
            ctx.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        let position = |p: egui::Pos2| -> (u16, u16) {
            (
                ((p.x - response.rect.left()) / response.rect.width() * w as f32)
                    .clamp(0., w.saturating_sub(1) as f32) as u16,
                ((p.y - response.rect.top()) / response.rect.height() * h as f32)
                    .clamp(0., h.saturating_sub(1) as f32) as u16,
            )
        };
        let mut outbound = Vec::new();
        if self.remote_focus {
            let current = ctx.input(|i| i.modifiers);
            for packet in self
                .keyboard
                .translate(&events, current, self.prefs.clipboard)
            {
                outbound.extend(packet);
            }
        }
        let mut pending_move = None;
        for event in events {
            if !matches!(event, egui::Event::PointerMoved(_))
                && let Some((x, y)) = pending_move.take()
            {
                outbound.extend(rfb::pointer(x, y, self.pointer_mask));
            }
            match event {
                egui::Event::PointerMoved(p) if hovered || self.pointer_mask != 0 => {
                    let (x, y) = position(p);
                    self.last_pointer = (x, y);
                    pending_move = Some((x, y));
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if hovered || self.pointer_mask != 0 => {
                    let bit = match button {
                        egui::PointerButton::Primary => 1,
                        egui::PointerButton::Middle => 2,
                        egui::PointerButton::Secondary => 4,
                        _ => 0,
                    };
                    if pressed {
                        self.pointer_mask |= bit;
                    } else {
                        self.pointer_mask &= !bit;
                    }
                    let (x, y) = position(pos);
                    self.last_pointer = (x, y);
                    outbound.extend(rfb::pointer(x, y, self.pointer_mask));
                }
                egui::Event::MouseWheel { unit, delta, .. } if hovered => {
                    if let Some(p) = ctx.pointer_hover_pos() {
                        self.last_pointer = position(p);
                    }
                    let bytes = self.wheel.translate(
                        (unit, delta),
                        response.rect.size(),
                        self.prefs.wheel_speed,
                        self.last_pointer,
                        self.pointer_mask,
                    );
                    outbound.extend(bytes);
                }
                _ => {}
            }
        }
        if let Some((x, y)) = pending_move {
            outbound.extend(rfb::pointer(x, y, self.pointer_mask));
        }
        if !outbound.is_empty() {
            s.send(outbound);
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.help {
            let help = egui::Modal::new(egui::Id::new("connection_help"))
                .backdrop_color(Color32::from_black_alpha(45))
                .frame(
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0_f32, LINE))
                        .corner_radius(16)
                        .inner_margin(24),
                )
                .show(ctx, |ui| {
                    ui.set_width(344.);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Image::new(&self.brand_icon)
                                .fit_to_exact_size(Vec2::splat(32.))
                                .alt_text("Icona Deskora"),
                        );
                        ui.label(emphasis("Deskora", 20.));
                    });
                    ui.add_space(8.);
                    ui.label(emphasis("Come collegarti", 23.));
                    ui.add_space(12.);
                    ui.label(emphasis("Sul computer remoto", 15.));
                    ui.label("Avvia un server VNC e tieni pronti indirizzo e password.");
                    ui.add_space(6.);
                    ui.label(emphasis("In questa finestra", 15.));
                    ui.label("Inserisci l’indirizzo e la password, poi premi Connetti.");
                    ui.add_space(6.);
                    ui.separator();
                    ui.label(
                        RichText::new(
                            "Usa una rete fidata o una VPN: VNC classico non cifra la connessione.",
                        )
                        .size(13.)
                        .color(MUTED),
                    );
                    egui::CollapsingHeader::new("Porte e scorciatoie").show(ui, |ui| {
                        ui.label("Porta standard: 5900. Puoi cambiarla in Opzioni.");
                        ui.label("Ctrl+Alt+Maiusc+F: attiva o chiude lo schermo intero.");
                    });
                    ui.add_space(8.);
                    if ui
                        .add_sized([ui.available_width(), 40.], egui::Button::new("Chiudi"))
                        .clicked()
                    {
                        ui.close();
                    }
                });
            if help.should_close() {
                self.help = false;
            }
        }
        if let Some(notice) = self.notice.clone() {
            egui::Area::new("notice".into())
                .anchor(egui::Align2::CENTER_BOTTOM, [0., -48.])
                .show(ctx, |ui| {
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0_f32, LINE))
                        .corner_radius(8)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(notice);
                                if ui.small_button("Chiudi").clicked() {
                                    self.notice = None;
                                }
                            });
                        });
                });
        }
    }
    fn screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.args.screenshot.clone() else {
            return;
        };
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                for pixel in &image.pixels {
                    rgba.extend_from_slice(&pixel.to_array());
                }
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                match image::save_buffer(
                    &path,
                    &rgba,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => {
                        eprintln!("Screenshot: {e}");
                        std::process::exit(2);
                    }
                }
            }
        }
        let ready = self.args.connect.is_none() || self.texture.is_some() || self.error.is_some();
        if self.frames >= 120 && ready && !self.shot_requested {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.frames += 1;
        self.pump(ctx);
        if ctx.input_mut(|i| {
            i.consume_key(
                egui::Modifiers::CTRL | egui::Modifiers::ALT | egui::Modifiers::SHIFT,
                egui::Key::F,
            )
        }) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        let connected = self
            .session
            .as_ref()
            .is_some_and(|s| s.state.lock().unwrap().connected);
        if connected {
            self.desktop(ctx);
        } else {
            self.home(ctx);
        }
        self.dialogs(ctx);
        self.screenshot(ctx);
        if self.session.is_some() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.release_input();
        if let Some(s) = self.session.take() {
            s.stop();
        }
        self.save_preferences();
    }
}
fn primary(ui: &mut egui::Ui, label: &str, width: f32) -> egui::Response {
    let response = ui.add_sized(
        [width, 50.],
        egui::Button::new(emphasis(label, 16.).color(Color32::WHITE))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(10),
    );
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter().rect_filled(
            response.rect,
            10.,
            Color32::from_white_alpha(if response.is_pointer_button_down_on() {
                25
            } else {
                13
            }),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(3.),
            13.,
            Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
    let color = if response.enabled() {
        Color32::WHITE
    } else {
        Color32::from_white_alpha(100)
    };
    let center = response.rect.right_center() - Vec2::new(24., 0.);
    ui.painter().arrow(
        center - Vec2::new(7., 0.),
        Vec2::new(14., 0.),
        Stroke::new(1.6_f32, color),
    );
    response
}
fn emphasis_family() -> egui::FontFamily {
    egui::FontFamily::Name("emphasis".into())
}
fn setting_checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        for state in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
        ] {
            state.corner_radius = CornerRadius::same(3);
        }
        ui.checkbox(value, label)
    })
    .inner
}
fn emphasis(text: &str, size: f32) -> RichText {
    RichText::new(text)
        .family(emphasis_family())
        .size(size)
        .color(INK)
}
fn visibility_button(ui: &mut egui::Ui, visible: bool) -> egui::Response {
    let label = if visible {
        "Nascondi password"
    } else {
        "Mostra password"
    };
    let response = ui.add_sized([38., 48.], egui::Button::new("").frame(false));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), label)
    });
    let c = response.rect.center();
    let color = if response.hovered() || response.has_focus() {
        ACCENT
    } else {
        MUTED
    };
    let points = vec![
        c + Vec2::new(-10., 0.),
        c + Vec2::new(-5., -5.),
        c + Vec2::new(0., -6.),
        c + Vec2::new(5., -5.),
        c + Vec2::new(10., 0.),
        c + Vec2::new(5., 5.),
        c + Vec2::new(0., 6.),
        c + Vec2::new(-5., 5.),
    ];
    ui.painter().add(egui::Shape::closed_line(
        points,
        Stroke::new(1.5_f32, color),
    ));
    ui.painter()
        .circle_stroke(c, 2.5, Stroke::new(1.5_f32, color));
    if visible {
        ui.painter().line_segment(
            [c + Vec2::new(-9., 9.), c + Vec2::new(9., -9.)],
            Stroke::new(1.5_f32, color),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(1.),
            8.,
            Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response.on_hover_text(label)
}
fn monitor_icon(ui: &mut egui::Ui, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let screen = egui::Rect::from_min_size(
        rect.min + Vec2::new(1., 2.),
        Vec2::new(size - 2., size * 0.65),
    );
    ui.painter().rect_stroke(
        screen,
        3.,
        Stroke::new(1.7_f32, color),
        egui::StrokeKind::Inside,
    );
    let c = screen.center_bottom();
    ui.painter().line_segment(
        [c, c + Vec2::new(0., size * 0.15)],
        Stroke::new(1.7_f32, color),
    );
    ui.painter().line_segment(
        [
            c + Vec2::new(-size * 0.2, size * 0.15),
            c + Vec2::new(size * 0.2, size * 0.15),
        ],
        Stroke::new(1.7_f32, color),
    );
}
fn keysym(k: egui::Key, m: egui::Modifiers) -> Option<u32> {
    use egui::Key::*;
    Some(match k {
        ArrowLeft => 0xff51,
        ArrowUp => 0xff52,
        ArrowRight => 0xff53,
        ArrowDown => 0xff54,
        Escape => 0xff1b,
        Tab => 0xff09,
        Backspace => 0xff08,
        Enter => 0xff0d,
        Insert => 0xff63,
        Delete => 0xffff,
        Home => 0xff50,
        End => 0xff57,
        PageUp => 0xff55,
        PageDown => 0xff56,
        F1 => 0xffbe,
        F2 => 0xffbf,
        F3 => 0xffc0,
        F4 => 0xffc1,
        F5 => 0xffc2,
        F6 => 0xffc3,
        F7 => 0xffc4,
        F8 => 0xffc5,
        F9 => 0xffc6,
        F10 => 0xffc7,
        F11 => 0xffc8,
        F12 => 0xffc9,
        _ => {
            if m.ctrl || m.alt || m.command {
                let name = k.name();
                if name.len() == 1 {
                    name.as_bytes()[0].to_ascii_lowercase() as u32
                } else {
                    return None;
                }
            } else {
                return None;
            }
        }
    })
}
fn main() -> eframe::Result {
    let args = Args::read();
    let size = if args.screenshot.is_some() {
        args.screenshot_size.unwrap_or(HOME_SIZE)
    } else {
        HOME_SIZE
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/deskora-icon.png"))
        .expect("Deskora window icon must be a valid PNG");
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_min_inner_size(HOME_MIN_SIZE)
            .with_title("Deskora")
            .with_app_id("Deskora")
            .with_icon(icon),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };
    eframe::run_native(
        "Deskora",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, args)))),
    )
}

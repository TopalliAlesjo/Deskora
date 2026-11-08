//! Retains pressed keys until release, including when modifiers change first.
use crate::{keysym, rfb};
use eframe::egui::{Event, Key, Modifiers};
use std::collections::HashMap;

/// Converts native line/point/page deltas into RFB wheel press/release pairs.
#[derive(Default)]
pub struct RemoteWheel {
    remainder: eframe::egui::Vec2,
}
impl RemoteWheel {
    pub fn reset(&mut self) {
        self.remainder = eframe::egui::Vec2::ZERO;
    }

    pub fn translate(
        &mut self,
        event: (eframe::egui::MouseWheelUnit, eframe::egui::Vec2),
        viewport: eframe::egui::Vec2,
        speed: f32,
        position: (u16, u16),
        held: u8,
    ) -> Vec<u8> {
        use eframe::egui::MouseWheelUnit;
        let (unit, delta) = event;
        if !delta.is_finite() || delta == eframe::egui::Vec2::ZERO {
            return vec![];
        }
        let factor = match unit {
            MouseWheelUnit::Line => eframe::egui::Vec2::splat(1.),
            MouseWheelUnit::Point => eframe::egui::Vec2::splat(1. / 20.),
            MouseWheelUnit::Page => viewport / 20.,
        };
        let speed = if speed.is_finite() {
            speed.clamp(0.25, 32.)
        } else {
            3.
        };
        self.remainder += delta * factor * speed;
        let mut bytes = Vec::new();
        for (axis, positive, negative) in [(1, 8, 16), (0, 32, 64)] {
            let value = self.remainder[axis];
            let count = value.abs().trunc().min(64.) as usize;
            if count == 0 {
                continue;
            }
            self.remainder[axis] = if value.abs() > 64. {
                0.
            } else {
                value - count as f32 * value.signum()
            };
            let bit = if value > 0. { positive } else { negative };
            for _ in 0..count {
                bytes.extend(rfb::pointer(position.0, position.1, held | bit));
                bytes.extend(rfb::pointer(position.0, position.1, held));
            }
        }
        bytes
    }
}

#[derive(Default)]
pub struct RemoteKeyboard {
    held: HashMap<Key, u32>,
    modifiers: [bool; 3],
    pending: Option<Key>,
}
impl RemoteKeyboard {
    fn sync(&mut self, m: Modifiers, out: &mut Vec<Vec<u8>>) {
        for (i, down) in [m.shift, m.ctrl, m.alt].into_iter().enumerate() {
            if down != self.modifiers[i] {
                out.push(rfb::key([0xffe1, 0xffe3, 0xffe9][i], down));
                self.modifiers[i] = down;
            }
        }
    }
    fn shortcut(&self, key: u32, out: &mut Vec<Vec<u8>>) {
        if !self.modifiers[1] {
            out.push(rfb::key(0xffe3, true));
        }
        out.push(rfb::key(key, true));
        out.push(rfb::key(key, false));
        if !self.modifiers[1] {
            out.push(rfb::key(0xffe3, false));
        }
    }
    pub fn translate(
        &mut self,
        events: &[Event],
        current: Modifiers,
        clipboard: bool,
    ) -> Vec<Vec<u8>> {
        let mut out = vec![];
        self.sync(current, &mut out);
        for event in events {
            match event {
                Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    self.sync(*modifiers, &mut out);
                    if !pressed {
                        if let Some(symbol) = self.held.remove(key) {
                            out.push(rfb::key(symbol, false));
                        }
                        if self.pending == Some(*key) {
                            self.pending = None;
                        }
                    } else if let Some(symbol) = self.held.get(key) {
                        if keysym(*key, *modifiers).is_some() {
                            out.push(rfb::key(*symbol, true));
                            self.pending = None;
                        } else {
                            self.pending = Some(*key);
                        }
                    } else if let Some(symbol) = keysym(*key, *modifiers) {
                        out.push(rfb::key(symbol, true));
                        self.held.insert(*key, symbol);
                        self.pending = None;
                    } else {
                        self.pending = Some(*key);
                    }
                }
                Event::Text(value) => {
                    let mut chars = value.chars().filter(|c| !c.is_control());
                    let first = chars.next();
                    let multi = chars.next().is_some();
                    if let Some(c) = first {
                        if !multi && let Some(key) = self.pending.take() {
                            let symbol = unicode(c);
                            out.push(rfb::key(symbol, true));
                            self.held.insert(key, symbol);
                        } else {
                            for c in value.chars().filter(|c| !c.is_control()) {
                                out.push(rfb::key(unicode(c), true));
                                out.push(rfb::key(unicode(c), false));
                            }
                        }
                    }
                }
                Event::Copy => self.shortcut(b'c' as u32, &mut out),
                Event::Cut => self.shortcut(b'x' as u32, &mut out),
                Event::Paste(value) => {
                    if clipboard {
                        out.push(rfb::cut_text(value));
                    }
                    self.shortcut(b'v' as u32, &mut out);
                }
                _ => {}
            }
        }
        self.sync(current, &mut out);
        out
    }
    pub fn release(&mut self) -> Vec<Vec<u8>> {
        let mut out: Vec<_> = self.held.drain().map(|(_, k)| rfb::key(k, false)).collect();
        for (i, down) in self.modifiers.iter_mut().enumerate() {
            if *down {
                out.push(rfb::key([0xffe1, 0xffe3, 0xffe9][i], false));
                *down = false;
            }
        }
        self.pending = None;
        out
    }
}
fn unicode(c: char) -> u32 {
    if c as u32 <= 255 {
        c as u32
    } else {
        0x01000000 | c as u32
    }
}

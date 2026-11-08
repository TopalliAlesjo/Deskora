use super::*;

fn dropdown_mark(ui: &egui::Ui, response: &egui::Response) {
    let center = response.rect.right_center() - Vec2::new(12., 0.);
    ui.painter().add(egui::Shape::line(
        vec![
            center + Vec2::new(-3., -2.),
            center + Vec2::new(0., 1.),
            center + Vec2::new(3., -2.),
        ],
        Stroke::new(1.5_f32, INK),
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DisplayMode {
    Fill,
    Fit,
    Actual,
}
impl DisplayMode {
    fn label(self) -> &'static str {
        match self {
            Self::Fill => "Riempi",
            Self::Fit => "Adatta",
            Self::Actual => "100%",
        }
    }
    pub(super) fn size(self, native: Vec2, available: Vec2) -> Vec2 {
        match self {
            Self::Fill => available,
            Self::Fit => native * (available.x / native.x).min(available.y / native.y),
            Self::Actual => native,
        }
    }
}

impl App {
    pub(super) fn desktop(&mut self, ctx: &egui::Context) {
        let (name, w, h, encoding, decode_ms, coalesced, continuous) = {
            let state = self.session.as_ref().unwrap().state.lock().unwrap();
            (
                state.name.clone(),
                state.width,
                state.height,
                state.encoding.clone(),
                state.decode_ms,
                state.coalesced_updates,
                state.continuous,
            )
        };
        let diagnostics = format!(
            "{w} × {h} · {encoding}\n{:.1} aggiornamenti/s dal server\nDecodifica ultimo aggiornamento: {decode_ms:.2} ms\nAggiornamenti accorpati: {coalesced}\nFlusso: {}",
            self.fps,
            if continuous {
                "continuo"
            } else {
                "richieste anticipate"
            }
        );
        egui::TopBottomPanel::top("session_toolbar")
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .inner_margin(egui::Margin::symmetric(12, 5)),
            )
            .show(ctx, |ui| {
                let wide = ui.available_width() >= 900.;
                let show_status_text = ui.available_width() >= 680.;
                let right_width = if wide {
                    235.
                } else if show_status_text {
                    136.
                } else {
                    70.
                };
                let left_width = (ui.available_width() - right_width - 8.).max(100.);
                ui.spacing_mut().item_spacing = Vec2::new(8., 4.);
                ui.spacing_mut().interact_size.y = 32.;
                ui.spacing_mut().button_padding = Vec2::new(10., 6.);
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(left_width, 32.),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            let name_width = if wide {
                                152.
                            } else {
                                (left_width - 284.).clamp(50., 120.)
                            };
                            ui.add_sized(
                                [name_width, 32.],
                                egui::Label::new(emphasis(&name, 14.)).truncate(),
                            )
                            .on_hover_text(&name);
                            ui.separator();
                            let display_button = ui
                                .menu_button(format!("{}    ", self.display_mode.label()), |ui| {
                                    for (mode, label) in [
                                        (DisplayMode::Fill, "Riempi finestra"),
                                        (DisplayMode::Fit, "Mantieni proporzioni"),
                                        (DisplayMode::Actual, "Dimensione originale · 100%"),
                                    ] {
                                        if ui
                                            .selectable_value(&mut self.display_mode, mode, label)
                                            .clicked()
                                        {
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("Dimensioni dello schermo remoto");
                            dropdown_mark(ui, &display_button);
                            let options_open = egui::Popup::is_id_open(
                                ctx,
                                egui::Id::new(crate::session_options::POPUP_ID),
                            );
                            let options_button = ui.add(
                                egui::Button::new("Opzioni    ")
                                    .selected(options_open)
                                    .fill(if options_open { SOFT } else { BG }),
                            );
                            dropdown_mark(ui, &options_button);
                            self.session_options(
                                ctx,
                                &options_button,
                                &crate::session_options::SessionDetails {
                                    width: w,
                                    height: h,
                                    encoding: &encoding,
                                    fps: self.fps,
                                    decode_ms,
                                    coalesced,
                                    continuous,
                                },
                            );
                            let fullscreen = if wide {
                                if self.fullscreen {
                                    "Esci da schermo intero"
                                } else {
                                    "Schermo intero"
                                }
                            } else {
                                "⛶"
                            };
                            let full = ui
                                .add(
                                    egui::Button::new(fullscreen)
                                        .fill(BG)
                                        .min_size(Vec2::splat(32.)),
                                )
                                .on_hover_text("Schermo intero · Ctrl+Alt+Maiusc+F");
                            full.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Schermo intero",
                                )
                            });
                            if full.clicked() {
                                self.fullscreen = !self.fullscreen;
                                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                                    self.fullscreen,
                                ));
                            }
                            let disconnect = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(if wide { "Disconnetti" } else { "×" })
                                            .color(Color32::from_rgb(166, 54, 43)),
                                    )
                                    .fill(BG)
                                    .min_size(Vec2::splat(32.)),
                                )
                                .on_hover_text("Disconnetti");
                            disconnect.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    true,
                                    "Disconnetti",
                                )
                            });
                            if disconnect.clicked() {
                                self.disconnect(ctx);
                            }
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(if wide { "Aiuto" } else { "?" })
                                    .fill(BG)
                                    .min_size(Vec2::splat(32.)),
                            )
                            .on_hover_text("Aiuto")
                            .clicked()
                        {
                            self.help = true;
                        }
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(Vec2::splat(8.), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3., ACCENT);
                            if show_status_text {
                                ui.label(RichText::new("Connesso").size(13.).color(ACCENT));
                            }
                        })
                        .response
                        .on_hover_text(&diagnostics);
                        if wide {
                            ui.label(RichText::new(format!("{w} × {h}")).size(12.).color(MUTED))
                                .on_hover_text(&diagnostics);
                        }
                    });
                });
            });
        if self.session.is_none() {
            return;
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(25, 31, 33))
                    .inner_margin(0),
            )
            .show(ctx, |ui| {
                let Some(texture) = self.texture.as_ref().map(|t| t.id()) else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("In arrivo il primo schermo…").color(Color32::WHITE),
                        );
                    });
                    return;
                };
                let native = Vec2::new(w as f32, h as f32);
                let available = ui.available_rect_before_wrap();
                let desired = self.display_mode.size(native, available.size());
                let response = if self.display_mode == DisplayMode::Actual {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .scroll_source(egui::scroll_area::ScrollSource {
                            scroll_bar: true,
                            drag: false,
                            mouse_wheel: false,
                        })
                        .show(ui, |ui| {
                            let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
                            ui.painter().image(
                                texture,
                                rect,
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                                Color32::WHITE,
                            );
                            ui.interact(
                                rect,
                                egui::Id::new("remote_screen"),
                                egui::Sense::click_and_drag(),
                            )
                        })
                        .inner
                } else {
                    let rect = egui::Rect::from_center_size(available.center(), desired);
                    ui.painter().image(
                        texture,
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                        Color32::WHITE,
                    );
                    ui.interact(
                        rect,
                        egui::Id::new("remote_screen"),
                        egui::Sense::click_and_drag(),
                    )
                };
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Image, true, "Schermo remoto")
                });
                if response.clicked() || response.drag_started() {
                    response.request_focus();
                    self.remote_focus = true;
                }
                let focused = response.has_focus() && ctx.input(|i| i.focused);
                if self.remote_focus && !focused {
                    self.release_input();
                }
                self.remote_focus = focused;
                if !self.prefs.view_only {
                    self.remote_input(ctx, &response, w, h);
                }
                if response.hovered()
                    && let Some(cursor) = &self.cursor_texture
                {
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                    if let Some(position) = ctx.pointer_hover_pos() {
                        let scale = response.rect.size() / native;
                        ui.painter().image(
                            cursor.id(),
                            egui::Rect::from_min_size(
                                position - self.cursor_hot * scale,
                                self.cursor_size * scale,
                            ),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                            Color32::WHITE,
                        );
                    }
                }
            });
    }
}

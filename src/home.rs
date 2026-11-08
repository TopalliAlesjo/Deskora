use super::*;

impl App {
    pub(super) fn home(&mut self, ctx: &egui::Context) {
        let content_height = egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .show(ctx, |ui| {
                let scroll = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let width = ui.available_width().min(440.);
                        let left = ((ui.available_width() - width) * 0.5).max(0.);
                        ui.horizontal(|ui| {
                            ui.add_space(left);
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 0.),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(width);
                                    ui.add_space(20.);
                                    ui.horizontal(|ui| {
                                        ui.label(emphasis("Collegati a un computer", 24.));
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .add(
                                                        egui::Button::new("Aiuto")
                                                            .frame(true)
                                                            .fill(BG)
                                                            .min_size(Vec2::new(56., 32.)),
                                                    )
                                                    .clicked()
                                                {
                                                    self.help = true;
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(6.);
                                    self.connection_form(ui, ctx, width);
                                    ui.add_space(16.);
                                    self.recent_connections(ui);
                                    ui.add_space(20.);
                                },
                            );
                        });
                    });
                scroll.content_size.y
            })
            .inner;
        self.fit_home_height(ctx, content_height);
    }

    /// Makes the window exactly as tall as the start screen, so no empty area is left below.
    /// It follows the content (password drawer, options, recents) only when the content
    /// changes, so a manual resize is kept until then.
    fn fit_home_height(&mut self, ctx: &egui::Context, content_height: f32) {
        if self.args.screenshot_size.is_some()
            || content_height <= 0.
            || (content_height - self.fitted_height).abs() < 1.
        {
            return;
        }
        let (width, maximized, monitor) = ctx.input(|i| {
            (
                i.viewport().inner_rect.map(|r| r.width()),
                i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true),
                i.viewport().monitor_size,
            )
        });
        let Some(width) = width else {
            return;
        };
        // The first fit (startup, back from a session) stays centered; later ones grow
        // downward, like a sheet opening below the address.
        let anchor = if self.fitted_height == 0. {
            Anchor::Center
        } else {
            Anchor::Top
        };
        self.fitted_height = content_height;
        if maximized {
            return;
        }
        let max_height = monitor.map_or(900., |m| (m.y * 0.85).max(HOME_MIN_SIZE[1]));
        let height = content_height.round().clamp(HOME_MIN_SIZE[1], max_height);
        resize_window(ctx, [width, height], anchor);
    }

    /// Opens the password drawer below the address and moves the focus into it.
    fn open_password(&mut self, ctx: &egui::Context) {
        self.password_open = true;
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("remote_password")));
    }

    fn connection_form(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, width: f32) {
        let busy = self.session.is_some();
        let padding = 20;
        if self.host.trim().is_empty() && !busy {
            self.password_open = false;
        }
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(16)
            .inner_margin(padding)
            .show(ui, |ui| {
                ui.set_width((width - 2. * padding as f32 - 2.).max(200.));
                ui.spacing_mut().item_spacing.y = 8.;
                let mut submit = false;
                let mut proceed = false;
                ui.add_enabled_ui(!busy, |ui| {
                    let host_label = ui.label(emphasis("Indirizzo del computer", 14.));
                    let host = ui
                        .add_sized(
                            [ui.available_width(), 44.],
                            egui::TextEdit::singleline(&mut self.host)
                                .id(egui::Id::new("remote_address"))
                                .hint_text("es. 192.168.1.10")
                                .background_color(BG)
                                .font(egui::FontId::proportional(17.))
                                .margin(egui::Margin::symmetric(14, 11)),
                        )
                        .labelled_by(host_label.id);
                    if self.frames == 2 && self.args.screenshot.is_none() {
                        host.request_focus();
                    }
                    proceed |= host.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                    let mut drawer =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ctx,
                            egui::Id::new("password_drawer"),
                            false,
                        );
                    drawer.set_open(self.password_open);
                    drawer.show_body_unindented(ui, |ui| {
                        ui.add_space(6.);
                        let password_label = ui
                            .allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), 20.),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    let label = ui.label(emphasis("Password", 14.));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.label(
                                                RichText::new("Vuota se non richiesta")
                                                    .size(13.)
                                                    .color(MUTED),
                                            );
                                        },
                                    );
                                    label
                                },
                            )
                            .inner;
                        ui.horizontal(|ui| {
                            let password = ui
                                .add_sized(
                                    [ui.available_width() - 48., 44.],
                                    egui::TextEdit::singleline(&mut self.password)
                                        .id(egui::Id::new("remote_password"))
                                        .password(!self.show_password)
                                        .hint_text("Password del computer")
                                        .background_color(BG)
                                        .margin(egui::Margin::symmetric(14, 11)),
                                )
                                .labelled_by(password_label.id);
                            submit |= password.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if visibility_button(ui, self.show_password).clicked() {
                                self.show_password = !self.show_password;
                            }
                        });
                    });
                    ui.add_space(6.);
                    let label = if busy {
                        "Connessione…"
                    } else if self.password_open {
                        "Connetti"
                    } else {
                        "Continua"
                    };
                    if primary(ui, label, ui.available_width()).clicked() {
                        if self.password_open {
                            submit = true;
                        } else {
                            proceed = true;
                        }
                    }
                });
                if proceed && !busy {
                    if self.host.trim().is_empty() {
                        self.error = Some("Inserisci l’indirizzo del computer.".into());
                        ctx.memory_mut(|m| m.request_focus(egui::Id::new("remote_address")));
                    } else {
                        self.error = None;
                        self.open_password(ctx);
                    }
                }
                if submit && !busy {
                    self.connect(ctx, false);
                }
                if busy {
                    ui.add_space(4.);
                    ui.horizontal_wrapped(|ui| {
                        ui.spinner();
                        if let Some(session) = &self.session {
                            ui.label(
                                RichText::new(&session.state.lock().unwrap().status)
                                    .size(14.)
                                    .color(MUTED),
                            );
                        }
                        if ui.button("Annulla").clicked() {
                            self.disconnect(ctx);
                        }
                    });
                }
                if let Some(error) = &self.error {
                    let summary = if error.starts_with("Il computer non risponde.") {
                        "Controlla indirizzo, server VNC e rete."
                    } else if error.starts_with("Accesso rifiutato.") {
                        "Accesso rifiutato. Controlla la password VNC."
                    } else if error.starts_with("Connessione interrotta:") {
                        "La connessione si è interrotta. Riprova."
                    } else if error.starts_with("Impossibile ascoltare sulla porta") {
                        "La porta di ascolto non è disponibile. Prova a cambiarla."
                    } else {
                        error.as_str()
                    };
                    ui.add_space(4.);
                    egui::Frame::new()
                        .fill(Color32::from_rgb(252, 239, 235))
                        .corner_radius(9)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width((ui.available_width() - 2.).max(100.));
                            ui.label(
                                emphasis("Impossibile collegarsi", 14.)
                                    .color(Color32::from_rgb(150, 48, 35)),
                            );
                            ui.label(RichText::new(summary).size(14.));
                            if summary != error {
                                egui::CollapsingHeader::new(
                                    RichText::new("Dettagli tecnici").size(13.).color(MUTED),
                                )
                                .show(ui, |ui| {
                                    ui.label(RichText::new(error).size(13.));
                                });
                            }
                        });
                }
                ui.add_space(4.);
                ui.separator();
                ui.horizontal(|ui| {
                    let label = if self.options {
                        "−  Opzioni"
                    } else {
                        "+  Opzioni"
                    };
                    if ui
                        .add(
                            egui::Button::new(RichText::new(label).color(ACCENT).size(14.))
                                .frame(true)
                                .selected(self.options)
                                .fill(if self.options { SOFT } else { BG })
                                .min_size(Vec2::new(96., 32.)),
                        )
                        .clicked()
                    {
                        self.options = !self.options;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("Porta {}", self.port_text))
                                .size(13.)
                                .color(MUTED),
                        );
                    });
                });
                if self.options {
                    ui.add_space(8.);
                    self.advanced(ui, ctx, busy);
                }
            });
    }

    fn recent_connections(&mut self, ui: &mut egui::Ui) {
        if self.prefs.recent.is_empty() {
            ui.horizontal(|ui| {
                monitor_icon(ui, 18., MUTED);
                ui.label(
                    RichText::new("I tuoi computer recenti appariranno qui.")
                        .size(14.)
                        .color(MUTED),
                );
            });
            return;
        }
        let enabled = self.session.is_none();
        ui.horizontal(|ui| {
            ui.label(emphasis("Recenti", 14.).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(RichText::new("Svuota").size(13.).color(MUTED))
                            .frame_when_inactive(false),
                    )
                    .on_hover_text("Cancella l’elenco dei computer recenti")
                    .clicked()
                {
                    self.prefs.recent.clear();
                    self.save_preferences();
                }
            });
        });
        let recents = self.prefs.recent.clone();
        let mut chosen = None;
        egui::Frame::new()
            .fill(Color32::WHITE)
            .stroke(Stroke::new(1.0_f32, LINE))
            .corner_radius(14)
            .inner_margin(4)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.;
                let row_width = ui.available_width();
                for recent in &recents {
                    let selected = self.password_open
                        && self.host == recent.host
                        && self.port_text == recent.port.to_string();
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(row_width, 44.),
                        if enabled {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        },
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            enabled,
                            format!("{} porta {}", recent.host, recent.port),
                        )
                    });
                    let hovered = enabled && response.hovered();
                    let fill = if selected {
                        SOFT
                    } else if hovered {
                        BG
                    } else {
                        Color32::WHITE
                    };
                    ui.painter().rect_filled(rect, 10., fill);
                    if selected || response.has_focus() {
                        ui.painter().rect_stroke(
                            rect,
                            10.,
                            Stroke::new(1.0_f32, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let mut row = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink2(Vec2::new(12., 0.)))
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    );
                    let c = row
                        .allocate_exact_size(Vec2::new(10., 20.), egui::Sense::hover())
                        .0
                        .center();
                    let color = if selected || hovered { ACCENT } else { MUTED };
                    row.painter().line(
                        vec![
                            c + Vec2::new(-2., -5.),
                            c + Vec2::new(3., 0.),
                            c + Vec2::new(-2., 5.),
                        ],
                        Stroke::new(1.6_f32, color),
                    );
                    row.label(
                        RichText::new(format!("Porta {}", recent.port))
                            .size(13.)
                            .color(MUTED),
                    );
                    row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        monitor_icon(ui, 18., if selected { ACCENT } else { MUTED });
                        ui.add(
                            egui::Label::new(emphasis(&recent.host, 15.))
                                .truncate()
                                .selectable(false),
                        );
                    });
                    if response
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        chosen = Some(recent.clone());
                    }
                }
            });
        if let Some(recent) = chosen {
            self.host = recent.host;
            self.port_text = recent.port.to_string();
            self.password.clear();
            self.error = None;
            self.open_password(ui.ctx());
        }
    }

    fn advanced(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, busy: bool) {
        let before = (
            self.port_text.clone(),
            self.listen_text.clone(),
            self.prefs.quality,
            self.prefs.view_only,
            self.prefs.clipboard,
            self.prefs.shared,
        );
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let label = ui.label(emphasis("Porta", 14.));
                ui.add_sized(
                    [90., 38.],
                    egui::TextEdit::singleline(&mut self.port_text)
                        .char_limit(5)
                        .margin(egui::Margin::symmetric(10, 10)),
                )
                .labelled_by(label.id);
                ui.label(RichText::new("Standard: 5900").size(13.).color(MUTED));
            });
            ui.add_space(8.);
            ui.label(emphasis("Qualità immagine", 14.));
            ui.horizontal_wrapped(|ui| {
                for quality in [Quality::Sharp, Quality::Balanced, Quality::Fluid] {
                    ui.selectable_value(&mut self.prefs.quality, quality, quality.label());
                }
            });
            ui.label(
                RichText::new(match self.prefs.quality {
                    Quality::Sharp => "Tutti i dettagli, senza perdita.",
                    Quality::Balanced => "Buon dettaglio, meno dati.",
                    Quality::Fluid => "Compressione leggera, per reti veloci.",
                })
                .size(13.)
                .color(MUTED),
            );
            ui.add_space(6.);
            setting_checkbox(ui, &mut self.prefs.view_only, "Sola visione")
                .on_hover_text("Disattiva il controllo di mouse e tastiera");
            setting_checkbox(ui, &mut self.prefs.clipboard, "Condividi appunti");
            setting_checkbox(ui, &mut self.prefs.shared, "Consenti altre connessioni")
                .on_hover_text("Permetti ad altri viewer di collegarsi allo stesso computer");
            ui.add_space(8.);
            ui.separator();
            egui::CollapsingHeader::new("Connessione in ascolto").show(ui, |ui| {
                ui.label(
                    RichText::new("Il computer remoto si collega a te.")
                        .size(13.)
                        .color(MUTED),
                );
                ui.horizontal_wrapped(|ui| {
                    let label = ui.label("Porta");
                    ui.add_sized(
                        [86., 38.],
                        egui::TextEdit::singleline(&mut self.listen_text)
                            .char_limit(5)
                            .margin(egui::Margin::symmetric(10, 10)),
                    )
                    .labelled_by(label.id);
                    if ui.button("Avvia ascolto").clicked() {
                        self.connect(ctx, true);
                    }
                });
            });
            if ui
                .add(
                    egui::Button::new(RichText::new("Ripristina opzioni").size(13.).color(MUTED))
                        .frame_when_inactive(false),
                )
                .clicked()
            {
                self.port_text = DEFAULT_PORT.to_string();
                self.listen_text = LISTEN_PORT.to_string();
                self.prefs.quality = Quality::Sharp;
                self.prefs.view_only = false;
                self.prefs.clipboard = true;
                self.prefs.shared = true;
            }
        });
        let after = (
            self.port_text.clone(),
            self.listen_text.clone(),
            self.prefs.quality,
            self.prefs.view_only,
            self.prefs.clipboard,
            self.prefs.shared,
        );
        if before != after {
            self.save_preferences();
        }
    }
}

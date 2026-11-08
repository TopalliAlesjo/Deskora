use super::*;

pub(super) const POPUP_ID: &str = "session_options_panel";

pub(super) struct SessionDetails<'a> {
    pub width: usize,
    pub height: usize,
    pub encoding: &'a str,
    pub fps: f64,
    pub decode_ms: f64,
    pub coalesced: u64,
    pub continuous: bool,
}

#[derive(Default)]
struct Actions {
    refresh: bool,
    attention: bool,
}

pub(super) fn settings_popover<R>(
    ctx: &egui::Context,
    button: &egui::Response,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Option<egui::InnerResponse<R>> {
    egui::Popup::from_toggle_button_response(button)
        .id(egui::Id::new(POPUP_ID))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .gap(6.)
        .width(338.)
        .frame(
            egui::Frame::new()
                .fill(Color32::WHITE)
                .stroke(Stroke::new(1.0_f32, LINE))
                .corner_radius(14)
                .inner_margin(18),
        )
        .show(|ui| {
            ui.set_width(300.);
            ui.set_max_height((ctx.content_rect().height() - 96.).max(240.));
            ui.spacing_mut().item_spacing = Vec2::new(8., 10.);
            ui.spacing_mut().interact_size.y = 34.;
            ui.spacing_mut().slider_width = 214.;
            ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
            let control_visuals = ui.visuals().widgets.clone();
            ui.visuals_mut().widgets.inactive.bg_fill = Color32::from_rgb(130, 149, 139);
            ui.visuals_mut().widgets.hovered.bg_fill = ACCENT;
            ui.visuals_mut().widgets.active.bg_fill = ACCENT;
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 96.).max(240.))
                .show(ui, |ui| {
                    ui.visuals_mut().widgets = control_visuals;
                    contents(ui)
                })
                .inner
        })
}

impl App {
    pub(super) fn session_options(
        &mut self,
        ctx: &egui::Context,
        button: &egui::Response,
        details: &SessionDetails<'_>,
    ) {
        let before = (
            self.prefs.view_only,
            self.prefs.quality,
            self.prefs.wheel_speed,
        );
        let shown = settings_popover(ctx, button, |ui| {
            options_content(ui, &mut self.prefs, details)
        });
        let after = (
            self.prefs.view_only,
            self.prefs.quality,
            self.prefs.wheel_speed,
        );
        if before.0 != after.0 {
            self.release_input();
        }
        if before.1 != after.1
            && let Some(session) = &self.session
        {
            session.send(rfb::set_encodings(self.prefs.quality));
        }
        if before.2 != after.2 {
            self.wheel.reset();
        }
        if before != after {
            self.save_preferences();
        }
        if let Some(shown) = shown
            && let Some(session) = &self.session
        {
            if shown.inner.refresh {
                let _ = session.tx.send(rfb::Command::Refresh(
                    details.width as u16,
                    details.height as u16,
                ));
            }
            if shown.inner.attention {
                let mut bytes = Vec::new();
                for (key, down) in [
                    (0xffe3, true),
                    (0xffe9, true),
                    (0xffff, true),
                    (0xffff, false),
                    (0xffe9, false),
                    (0xffe3, false),
                ] {
                    bytes.extend(rfb::key(key, down));
                }
                session.send(bytes);
            }
        }
    }
}

fn options_content(
    ui: &mut egui::Ui,
    prefs: &mut Preferences,
    details: &SessionDetails<'_>,
) -> Actions {
    ui.horizontal(|ui| {
        ui.label(emphasis("Opzioni", 20.));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_sized([30., 30.], egui::Button::new("×").fill(BG))
                .on_hover_text("Chiudi opzioni · Esc")
                .clicked()
            {
                ui.close();
            }
        });
    });
    ui.separator();
    setting_checkbox(ui, &mut prefs.view_only, "Sola visione")
        .on_hover_text("Disattiva il controllo di mouse e tastiera");
    ui.add_space(2.);
    ui.label(emphasis("Qualità immagine", 14.));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.;
        let width = (ui.available_width() - 12.) / 3.;
        for quality in [Quality::Sharp, Quality::Balanced, Quality::Fluid] {
            let selected = prefs.quality == quality;
            let label = quality.label();
            if ui
                .add_sized(
                    [width, 36.],
                    egui::Button::new(emphasis(label, 13.).color(if selected {
                        Color32::WHITE
                    } else {
                        INK
                    }))
                    .selected(selected)
                    .fill(if selected { ACCENT } else { BG })
                    .stroke(Stroke::new(
                        1.0_f32,
                        if selected {
                            ACCENT
                        } else {
                            Color32::from_rgb(130, 149, 139)
                        },
                    ))
                    .corner_radius(7),
                )
                .clicked()
            {
                prefs.quality = quality;
            }
        }
    });
    ui.label(
        RichText::new(match prefs.quality {
            Quality::Sharp => "Tutti i dettagli.",
            Quality::Balanced => "Meno dati, buon dettaglio.",
            Quality::Fluid => "Compressione leggera, per reti veloci.",
        })
        .size(12.)
        .color(MUTED),
    );
    ui.add_space(2.);
    ui.separator();
    ui.horizontal(|ui| {
        let label = ui.label(emphasis("Velocità rotella", 14.));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_sized(
                [78., 34.],
                egui::DragValue::new(&mut prefs.wheel_speed)
                    .range(0.5..=32.)
                    .speed(0.5)
                    .fixed_decimals(1)
                    .suffix("×"),
            )
            .labelled_by(label.id)
            .on_hover_text("Trascina il valore o fai doppio clic per scriverlo");
        });
    });
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.visuals_mut().widgets.inactive.bg_fill = LINE;
        ui.visuals_mut().selection.bg_fill = ACCENT;
        ui.add(
            egui::Slider::new(&mut prefs.wheel_speed, 0.5..=32.)
                .step_by(0.5)
                .trailing_fill(true)
                .handle_shape(egui::style::HandleShape::Circle)
                .show_value(false),
        )
        .widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Slider, true, "Velocità rotella")
        });
    });
    let mac = (prefs.wheel_speed - 20.).abs() < 0.01;
    if ui
        .add(
            egui::Button::new(
                RichText::new(if mac { "Mac attivo" } else { "Preset Mac" }).size(13.),
            )
            .fill(if mac { SOFT } else { BG })
            .min_size(Vec2::new(112., 32.)),
        )
        .on_hover_text("Imposta 20× per Condivisione schermo")
        .clicked()
    {
        prefs.wheel_speed = 20.;
    }
    ui.add_space(2.);
    ui.separator();
    ui.label(emphasis("Comandi", 14.));
    let mut actions = Actions::default();
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 8.) / 2.;
        actions.refresh = ui
            .add_sized([width, 36.], egui::Button::new("↻  Aggiorna").fill(BG))
            .on_hover_text("Richiedi l’intero schermo")
            .clicked();
        actions.attention = ui
            .add_enabled_ui(!prefs.view_only, |ui| {
                ui.add_sized(
                    [width, 36.],
                    egui::Button::new("Ctrl · Alt · Canc").fill(BG),
                )
            })
            .inner
            .clicked();
    });
    ui.add_space(2.);
    ui.separator();
    egui::CollapsingHeader::new(RichText::new("Dettagli connessione").size(13.).color(MUTED)).show(
        ui,
        |ui| {
            egui::Grid::new("connection_details")
                .num_columns(2)
                .spacing([14., 6.])
                .show(ui, |ui| {
                    for (label, value) in [
                        ("Schermo", format!("{} × {}", details.width, details.height)),
                        ("Codifica", details.encoding.to_owned()),
                        ("Aggiornamenti", format!("{:.1}/s", details.fps)),
                        ("Decodifica", format!("{:.2} ms", details.decode_ms)),
                        ("Accorpati", details.coalesced.to_string()),
                        (
                            "Flusso",
                            if details.continuous {
                                "Continuo"
                            } else {
                                "Su richiesta"
                            }
                            .to_owned(),
                        ),
                    ] {
                        ui.label(RichText::new(label).size(12.).color(MUTED));
                        ui.label(RichText::new(value).size(12.).color(INK));
                        ui.end_row();
                    }
                });
        },
    );
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        prefs: &mut Preferences,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800., 1000.),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let button = ui.button("Opzioni");
                    settings_popover(ctx, &button, |ui| {
                        options_content(
                            ui,
                            prefs,
                            &SessionDetails {
                                width: 1920,
                                height: 1080,
                                encoding: "ZRLE",
                                fps: 10.,
                                decode_ms: 1.,
                                coalesced: 0,
                                continuous: false,
                            },
                        )
                    });
                });
            },
        )
    }
    fn text_position(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(t) if t.galley.text() == text => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "Missing control: {text}; visible: {:?}",
                    output
                        .shapes
                        .iter()
                        .filter_map(|s| if let egui::Shape::Text(t) = &s.shape {
                            Some((t.galley.text(), t.pos))
                        } else {
                            None
                        })
                        .collect::<Vec<_>>()
                )
            })
    }
    fn click(ctx: &egui::Context, prefs: &mut Preferences, pos: egui::Pos2) -> egui::FullOutput {
        frame(
            ctx,
            prefs,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            ctx,
            prefs,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        frame(ctx, prefs, vec![])
    }
    #[test]
    fn settings_stay_open_after_edits_and_commands_but_close_outside_or_on_escape() {
        let ctx = egui::Context::default();
        let mut fonts = egui::FontDefinitions::default();
        fonts.families.insert(
            emphasis_family(),
            fonts.families[&egui::FontFamily::Proportional].clone(),
        );
        ctx.set_fonts(fonts);
        let mut prefs = Preferences::default();
        let first = frame(&ctx, &mut prefs, vec![]);
        let button = text_position(&first, "Opzioni");
        let mut output = click(&ctx, &mut prefs, button);
        assert!(egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)));
        let quality = text_position(&output, "Bilanciata");
        output = click(&ctx, &mut prefs, quality);
        assert_eq!(prefs.quality, Quality::Balanced);
        assert!(egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)));
        for label in [
            "Preset Mac",
            "Sola visione",
            "↻  Aggiorna",
            "Dettagli connessione",
        ] {
            let pos = text_position(&output, label);
            output = click(&ctx, &mut prefs, pos);
            assert!(
                egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)),
                "Closed after {label}"
            );
        }
        assert_eq!(prefs.wheel_speed, 20.);
        assert!(prefs.view_only);
        click(&ctx, &mut prefs, egui::pos2(760., 960.));
        assert!(!egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)));
        click(&ctx, &mut prefs, button);
        assert!(egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)));
        frame(
            &ctx,
            &mut prefs,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!egui::Popup::is_id_open(&ctx, egui::Id::new(POPUP_ID)));
    }
}

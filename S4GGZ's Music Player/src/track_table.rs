use crate::{
    bubble::BubbleSystem, centered_label, database::Track, fmt_time, label, TrackTableLayout, MUTED,
};
use eframe::egui::{self, Color32, RichText, Vec2};

pub(super) struct RowAction {
    title_clicked: bool,
    pub favorite: bool,
    pub delete: bool,
    pub rename: bool,
    pub add_to_playlist: bool,
    #[cfg(test)]
    pub right_clicked: bool,
    pub response: egui::Response,
}

impl RowAction {
    pub fn play_requested(&self) -> bool {
        !self.favorite && !self.delete && (self.title_clicked || self.response.clicked())
    }
}

pub(super) fn draw_track_row(
    row: &mut egui_extras::TableRow<'_, '_>,
    track: &Track,
    number: i64,
    playing: bool,
    layout: TrackTableLayout,
    allow_delete: bool,
) -> RowAction {
    let mut title_clicked = false;
    let mut favorite = false;
    let mut delete = false;
    let mut rename = false;
    let mut add_to_playlist = false;
    let mut right_clicked = false;
    row.set_selected(playing);
    row.col(|ui| {
        if playing {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    ui.max_rect().left_top() + Vec2::new(1.0, 10.0),
                    Vec2::new(2.0, 24.0),
                ),
                2,
                crate::theme::accent(ui.ctx()),
            );
        }
        centered_label(ui, number.to_string(), 12.0, MUTED);
    });
    row.col(|ui| {
        let response = ui.add(
            egui::Button::new(RichText::new(&track.title).strong().size(13.0))
                .fill(Color32::TRANSPARENT)
                .frame(false),
        );
        BubbleSystem::register_button(ui.ctx(), &response);
        title_clicked = response.clicked();
        right_clicked = ui.input(|input| {
            let clicked = input.pointer.button_clicked(egui::PointerButton::Secondary);
            clicked
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| response.rect.contains(position))
        });
        let popup = egui::Popup::menu(&response);
        let popup = if right_clicked {
            popup
                .open_memory(Some(egui::SetOpenCommand::Bool(true)))
                .at_pointer_fixed()
        } else {
            popup
        };
        popup.show(|ui| {
            if ui.button("Redenumește melodia").clicked() {
                rename = true;
                ui.close();
            }
            if ui.button("Adaugă la playlist").clicked() {
                add_to_playlist = true;
                ui.close();
            }
        });
    });
    if layout.show_artist {
        row.col(|ui| {
            ui.add(egui::Label::new(&track.artist).truncate());
        });
    }
    if layout.show_album {
        row.col(|ui| {
            ui.add(
                egui::Label::new(RichText::new(&track.album).size(12.0).color(MUTED)).truncate(),
            );
        });
    }
    row.col(|ui| {
        label(ui, fmt_time(track.duration_ms), 12.0, MUTED);
    });
    row.col(|ui| {
        let rect = egui::Rect::from_center_size(ui.max_rect().center(), Vec2::new(28.0, 22.0));
        let response = ui
            .put(
                rect,
                egui::Button::new(if track.is_favorite { "♥" } else { "♡" }),
            )
            .on_hover_text("Favorite");
        BubbleSystem::register_button(ui.ctx(), &response);
        favorite = response.clicked();
    });
    row.col(|ui| {
        delete = crate::compact_icon_button(
            ui,
            "trash",
            "Șterge definitiv fișierul audio",
            allow_delete,
        )
        .clicked();
    });
    let response = row
        .response()
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click dreapta pe titlu pentru redenumire și playlisturi");
    if response.rect.is_positive() {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                true,
                format!("Redă {}", track.title),
            )
        });
    }
    RowAction {
        title_clicked,
        favorite,
        delete,
        rename,
        add_to_playlist,
        #[cfg(test)]
        right_clicked,
        response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_extras::{Column, TableBuilder};

    fn sample() -> Track {
        Track {
            id: 1,
            file_path: "song.mp3".into(),
            title: "Melodie".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            album_id: 1,
            duration_ms: 90_000,
            bitrate: None,
            sample_rate: None,
            genre: String::new(),
            track_number: Some(1),
            is_favorite: false,
            is_missing: false,
            play_count: 0,
        }
    }

    fn render(ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> RowAction {
        let mut result = None;
        let _ = crate::test_support::run_ui(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 180.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                TableBuilder::new(ui)
                    .id_salt("row-click-test")
                    .sense(egui::Sense::click())
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(36.0))
                    .column(Column::exact(200.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(75.0))
                    .column(Column::exact(46.0))
                    .column(Column::exact(28.0))
                    .body(|body| {
                        body.rows(44.0, 1, |mut row| {
                            result = Some(draw_track_row(
                                &mut row,
                                &sample(),
                                1,
                                false,
                                TrackTableLayout {
                                    show_artist: true,
                                    show_album: true,
                                },
                                true,
                            ));
                        })
                    });
            },
        );
        result.unwrap()
    }

    #[test]
    fn number_title_artist_album_and_duration_play_but_heart_only_favorites() {
        for column in 0..7 {
            let ctx = egui::Context::default();
            let _ = render(&ctx, 0.0, vec![]);
            let first = render(&ctx, 0.01, vec![]);
            assert!(first.response.rect.is_positive());
            let spacing = ctx.style_of(egui::Theme::Dark).spacing.item_spacing.x;
            let widths = [36.0, 200.0, 120.0, 120.0, 75.0, 46.0, 28.0];
            let x = first.response.rect.left()
                + widths[..column].iter().sum::<f32>()
                + column as f32 * spacing
                + widths[column] * 0.5;
            let pos = egui::pos2(x, first.response.rect.center().y);
            let mut action = first;
            for (time, pressed) in [(0.1, true), (0.2, false)] {
                action = render(
                    &ctx,
                    time,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert_eq!(
                action.favorite,
                column == 5,
                "favorite action for column {column}"
            );
            assert_eq!(
                action.play_requested(),
                column < 5,
                "play action for column {column}"
            );
            assert_eq!(action.delete, column == 6);
        }
    }

    #[test]
    fn right_clicking_a_track_opens_rename_and_playlist_actions() {
        let ctx = egui::Context::default();
        let _ = render(&ctx, 0.0, vec![]);
        let first = render(&ctx, 0.01, vec![]);
        let point = egui::pos2(
            first.response.rect.left() + 36.0 + 8.0 + 12.0,
            first.response.rect.top() + 9.0,
        );
        let _ = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 180.0),
                )),
                time: Some(0.1),
                events: vec![
                    egui::Event::PointerMoved(point),
                    egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Secondary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                TableBuilder::new(ui)
                    .id_salt("row-click-test")
                    .sense(egui::Sense::click())
                    .column(Column::exact(36.0))
                    .column(Column::exact(200.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(75.0))
                    .column(Column::exact(46.0))
                    .column(Column::exact(28.0))
                    .body(|body| {
                        body.rows(44.0, 1, |mut row| {
                            let _ = draw_track_row(
                                &mut row,
                                &sample(),
                                1,
                                false,
                                TrackTableLayout {
                                    show_artist: true,
                                    show_album: true,
                                },
                                true,
                            );
                        });
                    });
            },
        );
        let mut right_clicked = false;
        let _ = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 180.0),
                )),
                time: Some(0.2),
                events: vec![egui::Event::PointerButton {
                    pos: point,
                    button: egui::PointerButton::Secondary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            },
            |ui| {
                TableBuilder::new(ui)
                    .id_salt("row-click-test")
                    .sense(egui::Sense::click())
                    .column(Column::exact(36.0))
                    .column(Column::exact(200.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(75.0))
                    .column(Column::exact(46.0))
                    .column(Column::exact(28.0))
                    .body(|body| {
                        body.rows(44.0, 1, |mut row| {
                            let action = draw_track_row(
                                &mut row,
                                &sample(),
                                1,
                                false,
                                TrackTableLayout {
                                    show_artist: true,
                                    show_album: true,
                                },
                                true,
                            );
                            right_clicked = action.right_clicked;
                        });
                    });
            },
        );
        assert!(right_clicked, "track row should receive a secondary click");
        let output = crate::test_support::run_ui(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(800.0, 180.0),
                )),
                time: Some(0.3),
                ..Default::default()
            },
            |ui| {
                TableBuilder::new(ui)
                    .id_salt("row-click-test")
                    .sense(egui::Sense::click())
                    .column(Column::exact(36.0))
                    .column(Column::exact(200.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(120.0))
                    .column(Column::exact(75.0))
                    .column(Column::exact(46.0))
                    .column(Column::exact(28.0))
                    .body(|body| {
                        body.rows(44.0, 1, |mut row| {
                            let _ = draw_track_row(
                                &mut row,
                                &sample(),
                                1,
                                false,
                                TrackTableLayout {
                                    show_artist: true,
                                    show_album: true,
                                },
                                true,
                            );
                        });
                    });
            },
        );
        let text = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(text.iter().any(|text| text == "Redenumește melodia"));
        assert!(text.iter().any(|text| text == "Adaugă la playlist"));
    }
}

use super::*;

impl Downloader {
    pub fn show_queue(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        frost: &TextureHandle,
        _db: &Database,
    ) {
        let mut remove = None;
        let mut retry = None;
        let panel = crate::glass_frame(ui, GLASS, 16, 12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            ui.label(RichText::new("Coada de descărcări").size(18.0).strong());
            ui.label(
                RichText::new(format!("{} în coadă · {} în așteptare", self.queue_len(), self.pending.len()))
                    .size(12.0)
                    .color(MUTED),
            );
            ui.add_space(12.0);
            egui::ScrollArea::vertical()
                .id_salt("download-queue-items")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(job) = &self.active {
                        queue_card(ui, |ui| {
                            section(ui, "SE DESCARCĂ");
                            job_labels(ui, job);
                            let progress = self.progress.lock().ok().map(|value| *value)
                                .filter(|value| value.is_finite() && (0.0..=100.0).contains(value));
                            if let Some(progress) = progress {
                                ui.add(
                                    egui::ProgressBar::new(progress / 100.0)
                                        .desired_width(ui.available_width())
                                        .fill(crate::theme::accent(ui.ctx())),
                                );
                            } else {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(RichText::new("Descărcare în curs...").small().color(MUTED));
                                });
                            }
                            let cancelling = self.download_cancel.load(std::sync::atomic::Ordering::Relaxed);
                            ui.horizontal_wrapped(|ui| {
                                if let Some(progress) = progress {
                                    let text = if progress > 0.0 && progress < 1.0 {
                                        "<1%".to_owned()
                                    } else {
                                        format!("{progress:.0}%")
                                    };
                                    ui.label(RichText::new(text).small().color(MUTED));
                                }
                                if ui.add_enabled(!cancelling, egui::Button::new(if cancelling {
                                    "Se anulează..."
                                } else {
                                    "Anulează"
                                })).clicked() {
                                    self.download_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                                }
                            });
                        });
                        ctx.request_repaint_after(Duration::from_millis(120));
                    } else if self.pending.is_empty() {
                        queue_card(ui, |ui| {
                            ui.label(RichText::new("Coada este liberă").strong());
                            ui.label(RichText::new("Alege o melodie și apasă Descarcă. Poți adăuga apoi oricâte alte melodii.").small().color(MUTED));
                        });
                    }
                    if !self.pending.is_empty() {
                        ui.add_space(10.0);
                        section(ui, "ÎN AȘTEPTARE");
                    }
                    for (index, job) in self.pending.iter().enumerate() {
                        ui.push_id(job.id, |ui| {
                            queue_card(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("{:02}", index + 1)).small().color(MUTED));
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if crate::compact_icon_button(ui, "close", "Elimină din coada de descărcare", true).clicked() {
                                            remove = Some(job.id);
                                        }
                                    });
                                });
                                job_labels(ui, job);
                            });
                        });
                    }
                    if !self.finished.is_empty() {
                        ui.add_space(12.0);
                        section(ui, "RECENTE");
                    }
                    for entry in &self.finished {
                        ui.push_id(("finished", entry.job.id), |ui| {
                            queue_card(ui, |ui| {
                                job_labels(ui, &entry.job);
                                match &entry.outcome {
                                    DownloadOutcome::Completed => {
                                        ui.label(RichText::new("Finalizată").small().color(crate::theme::accent(ui.ctx())));
                                    }
                                    DownloadOutcome::Failed(error) => {
                                        ui.label(RichText::new("Descărcare eșuată").small().color(Color32::from_rgb(255, 150, 130)))
                                            .on_hover_text(error);
                                        if ui.button("Reîncearcă").clicked() { retry = Some(entry.job.id); }
                                    }
                                    DownloadOutcome::Cancelled => {
                                        ui.label(RichText::new("Anulată").small().color(MUTED));
                                        if ui.button("Reîncearcă").clicked() { retry = Some(entry.job.id); }
                                    }
                                }
                            });
                        });
                    }
                    if !self.finished.is_empty() && ui.small_button("Curăță istoricul").clicked() {
                        self.finished.clear();
                    }
                });
        });
        crate::paint_frost_grain_in_rect(ui, frost, panel.response.rect.shrink(12.0));
        if let Some(id) = remove {
            self.remove_pending(id);
        }
        if let Some(id) = retry {
            self.retry(id);
        }
    }
}

fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(10.0)
            .strong()
            .color(crate::theme::accent(ui.ctx())),
    );
}

fn queue_card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    egui::Frame::new()
        .fill(crate::theme::tint(ui.ctx(), GLASS).gamma_multiply(0.6))
        .corner_radius(10)
        .inner_margin(10)
        .stroke(crate::glass_edge())
        .show(ui, |ui| {
            ui.set_width((width - 22.0).max(1.0));
            content(ui);
        });
    ui.add_space(6.0);
}

fn job_labels(ui: &mut egui::Ui, job: &DownloadJob) {
    ui.add(egui::Label::new(RichText::new(job.label()).strong()).truncate())
        .on_hover_text(&job.url);
    if !job.artist.is_empty() {
        ui.add(egui::Label::new(RichText::new(&job.artist).small().color(MUTED)).truncate());
    }
}

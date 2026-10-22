// THROWAWAY SPIKE (round 2): egui version of the hf2ollama Browse screen with a real design pass.
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Rect, RichText, Sense, Stroke, StrokeKind, TextStyle, Theme, UiBuilder, vec2,
};
use std::sync::Arc;

// ---------- design tokens (same values as the HTML mockup) ----------
#[derive(Clone, Copy)]
struct Tokens {
    bg: Color32,
    surface: Color32,
    surface2: Color32,
    line: Color32,
    fg: Color32,
    fg2: Color32,
    fg3: Color32,
    accent: Color32,
    accent_fg: Color32,
    accent_soft: Color32,
    ok: Color32,
    ok_soft: Color32,
    warn: Color32,
    warn_soft: Color32,
}
const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
const LIGHT: Tokens = Tokens {
    bg: rgb(0xee, 0xf0, 0xf2), surface: rgb(0xff, 0xff, 0xff), surface2: rgb(0xf5, 0xf6, 0xf8), line: rgb(0xdd, 0xe0, 0xe5),
    fg: rgb(0x16, 0x19, 0x1d), fg2: rgb(0x53, 0x5a, 0x64), fg3: rgb(0x8a, 0x91, 0x9b),
    accent: rgb(0x0b, 0x6e, 0x66), accent_fg: rgb(0xff, 0xff, 0xff), accent_soft: rgb(0xdf, 0xf1, 0xee),
    ok: rgb(0x1b, 0x7f, 0x3b), ok_soft: rgb(0xe0, 0xf3, 0xe5), warn: rgb(0x9a, 0x62, 0x00), warn_soft: rgb(0xfb, 0xef, 0xd3),
};
const DARK: Tokens = Tokens {
    bg: rgb(0x10, 0x13, 0x16), surface: rgb(0x17, 0x1b, 0x1f), surface2: rgb(0x1d, 0x22, 0x27), line: rgb(0x2a, 0x30, 0x36),
    fg: rgb(0xe7, 0xea, 0xee), fg2: rgb(0xa3, 0xab, 0xb5), fg3: rgb(0x6f, 0x77, 0x82),
    accent: rgb(0x3c, 0xc2, 0xb3), accent_fg: rgb(0x06, 0x20, 0x1d), accent_soft: rgb(0x14, 0x31, 0x2e),
    ok: rgb(0x5c, 0xc9, 0x7b), ok_soft: rgb(0x15, 0x30, 0x1c), warn: rgb(0xe5, 0xb0, 0x4a), warn_soft: rgb(0x33, 0x27, 0x0e),
};
fn tk(ui: &egui::Ui) -> Tokens {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semibold".into()))
}
fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data: [(&str, &[u8]); 4] = [
        ("hanken", include_bytes!("../assets/Hanken-400.ttf")),
        ("hanken-semi", include_bytes!("../assets/Hanken-600.ttf")),
        ("jb", include_bytes!("../assets/JetBrainsMono-400.ttf")),
        ("jb-medium", include_bytes!("../assets/JetBrainsMono-500.ttf")),
    ];
    for (name, bytes) in data {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "hanken".into());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "jb".into());
    fonts.families.insert(FontFamily::Name("semibold".into()), vec!["hanken-semi".into(), "hanken".into()]);
    ctx.set_fonts(fonts);
}

fn style_for(style: &mut egui::Style, t: &Tokens) {
    style.text_styles = [
        (TextStyle::Heading, semibold(22.0)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, mono(13.0)),
    ]
    .into();
    style.spacing.item_spacing = vec2(8.0, 8.0);
    style.spacing.button_padding = vec2(14.0, 7.0);
    style.interaction.selectable_labels = false;
    let v = &mut style.visuals;
    v.panel_fill = t.surface;
    v.window_fill = t.surface;
    v.extreme_bg_color = t.surface;
    v.faint_bg_color = t.surface2;
    v.text_edit_bg_color = Some(t.surface);
    v.hyperlink_color = t.accent;
    v.selection.bg_fill = t.accent_soft;
    v.selection.stroke = Stroke::new(1.0, t.accent);
    let r = CornerRadius::same(6);
    for (w, fill) in [
        (&mut v.widgets.noninteractive, t.surface),
        (&mut v.widgets.inactive, t.surface2),
        (&mut v.widgets.hovered, t.surface2),
        (&mut v.widgets.active, t.surface2),
    ] {
        w.corner_radius = r;
        w.weak_bg_fill = fill;
        w.bg_fill = fill;
        w.bg_stroke = Stroke::new(1.0, t.line);
        w.fg_stroke = Stroke::new(1.0, t.fg);
    }
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, t.fg);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, t.fg3);
    v.widgets.active.bg_stroke = Stroke::new(1.0, t.accent);
}

// ---------- sample data ----------
struct Model {
    owner: &'static str,
    name: &'static str,
    safetensors: bool,
    params: &'static str,
    arch: &'static str,
    dl: &'static str,
    upd: &'static str,
    ctx: &'static str,
    lic: &'static str,
    // (quant, size GB, quality note, quality 0..1, recommended)
    quants: &'static [(&'static str, f32, &'static str, f32, bool)],
}
const MODELS: &[Model] = &[
    Model { owner: "bartowski", name: "Qwen2.5-Coder-7B-Instruct-GGUF", safetensors: false, params: "7.6B", arch: "qwen2", dl: "412k", upd: "3 weeks ago", ctx: "32,768", lic: "apache-2.0",
        quants: &[("Q8_0", 8.10, "Near lossless", 1.0, false), ("Q6_K", 6.25, "Excellent", 0.96, false), ("Q5_K_M", 5.44, "Very good", 0.92, false),
                  ("Q4_K_M", 4.68, "Good, best balance", 0.86, true), ("Q3_K_M", 3.81, "Noticeable loss", 0.66, false), ("Q2_K", 3.02, "Heavy loss", 0.45, false)] },
    Model { owner: "mistralai", name: "Mistral-7B-Instruct-v0.3", safetensors: true, params: "7.25B", arch: "MistralForCausalLM", dl: "1.9M", upd: "5 months ago", ctx: "32,768", lic: "apache-2.0",
        quants: &[("f16", 14.5, "Full precision", 1.0, false), ("q8_0", 7.7, "Near lossless", 0.99, false), ("q4_K_M", 4.4, "Good, best balance", 0.86, true)] },
];

#[derive(Clone, Copy, PartialEq)]
enum Fit { Ok, Tight }
fn fit_of(gb: f32) -> (&'static str, Fit) {
    let need = gb + 1.5;
    if need <= 12.8 { ("Fits in VRAM", Fit::Ok) } else if need <= 16.0 { ("Tight fit", Fit::Tight) } else { ("Partial CPU offload", Fit::Tight) }
}

// ---------- small custom widgets ----------
fn caption(ui: &mut egui::Ui, text: &str) {
    let t = tk(ui);
    ui.label(RichText::new(text.to_uppercase()).font(semibold(11.0)).color(t.fg3));
}

fn tag(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new().fill(bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(7, 2)).show(ui, |ui| {
        ui.add(egui::Label::new(RichText::new(text).font(mono(11.0)).color(fg)).wrap_mode(egui::TextWrapMode::Extend));
    });
}

fn bar(ui: &mut egui::Ui, frac: f32, width: f32) {
    let t = tk(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, t.line);
    let mut fill = rect;
    fill.set_width(width * frac);
    ui.painter().rect_filled(fill, 3.0, t.accent);
}

fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> bool {
    let t = tk(ui);
    let b = egui::Button::new(RichText::new(text).font(semibold(14.0)).color(if enabled { t.accent_fg } else { t.fg3 }))
        .fill(if enabled { t.accent } else { t.line })
        .stroke(Stroke::NONE)
        .min_size(vec2(0.0, 36.0));
    ui.add_enabled(enabled, b).clicked()
}

fn small_button(ui: &mut egui::Ui, text: &str) -> bool {
    let t = tk(ui);
    ui.add(egui::Button::new(RichText::new(text).size(12.0).color(t.fg2)).min_size(vec2(0.0, 24.0))).clicked()
}

// ---------- app ----------
struct App {
    sel: usize,
    quant: usize,
    query: String,
    tab: usize,
    name: String,
    progress: Option<f32>,
    paused: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_fonts(&cc.egui_ctx);
        cc.egui_ctx.style_mut_of(Theme::Light, |s| style_for(s, &LIGHT));
        cc.egui_ctx.style_mut_of(Theme::Dark, |s| style_for(s, &DARK));
        let mut a = Self { sel: 0, quant: 3, query: "qwen coder".into(), tab: 0, name: String::new(), progress: None, paused: false };
        if std::env::var("HF2O_THEME").as_deref() == Ok("light") {
            cc.egui_ctx.set_theme(egui::ThemePreference::Light);
        }
        if std::env::var("HF2O_SEL").as_deref() == Ok("1") {
            a.select_model(1);
        }
        if std::env::var("HF2O_RUN").is_ok() {
            a.progress = Some(0.42);
            a.paused = true;
        }
        a.rename();
        a
    }
    fn rename(&mut self) {
        let m = &MODELS[self.sel];
        self.name = format!("{}:{}", m.name.to_lowercase().replace("-gguf", ""), m.quants[self.quant].0.to_lowercase());
    }
    fn select_model(&mut self, i: usize) {
        self.sel = i;
        self.quant = MODELS[i].quants.iter().position(|q| q.4).unwrap_or(0);
        self.rename();
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(p) = self.progress {
            if p < 1.0 && !self.paused {
                self.progress = Some((p + 0.003).min(1.0));
            }
            if p < 1.0 {
                ctx.request_repaint();
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let t = tk(ui);

        // top bar
        egui::Panel::top("top").frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(16, 10)).stroke(Stroke::new(1.0, t.line))).show(ui, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                ui.painter().rect_filled(r, 6.0, t.accent);
                ui.painter().circle_stroke(r.center(), 5.0, Stroke::new(2.0, t.accent_fg));
                ui.label(RichText::new("hf2ollama").font(semibold(16.0)));
                ui.add_space(8.0);
                // segmented nav
                egui::Frame::new().fill(t.surface2).stroke(Stroke::new(1.0, t.line)).corner_radius(8.0).inner_margin(2.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (i, name) in ["Browse", "Installed", "Settings"].iter().enumerate() {
                        let galley = ui.painter().layout_no_wrap((*name).into(), if i == self.tab { semibold(14.0) } else { FontId::proportional(14.0) }, if i == self.tab { t.fg } else { t.fg2 });
                        let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 26.0, 28.0), Sense::click());
                        if i == self.tab {
                            ui.painter().rect_filled(rect, 6.0, t.surface);
                            ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, t.line), StrokeKind::Inside);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, t.surface);
                        }
                        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.fg);
                        if resp.clicked() {
                            self.tab = i;
                        }
                    }
                });
                ui.add_space(8.0);
                ui.add_sized(vec2(360.0, 32.0), egui::TextEdit::singleline(&mut self.query).hint_text("Search Hugging Face models").margin(egui::Margin::symmetric(10, 7)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    egui::Frame::new().fill(t.surface2).stroke(Stroke::new(1.0, t.line)).corner_radius(99.0).inner_margin(egui::Margin::symmetric(10, 3)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 4.0, t.ok);
                            ui.label(RichText::new("Ollama 0.9.2 · localhost:11434").size(12.0).color(t.fg2));
                        });
                    });
                });
            });
        });

        // status bar
        egui::Panel::bottom("status").frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(16, 6)).stroke(Stroke::new(1.0, t.line))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let dim = |s: &str| RichText::new(s.to_string()).size(12.0).color(t.fg3);
                let val = |s: &str| RichText::new(s.to_string()).font(mono(11.5)).color(t.fg2);
                ui.label(dim("Staging folder"));
                ui.label(val("D:\\hf2ollama-staging"));
                ui.label(dim("(files deleted after each import)"));
                ui.add_space(14.0);
                ui.label(dim("Free"));
                ui.label(val("412 GB"));
                ui.add_space(14.0);
                ui.label(dim("Ollama store (reported by server)"));
                ui.label(val("E:\\ollama\\models"));
            });
        });

        // downloads dock
        egui::Panel::bottom("dock").frame(egui::Frame::new().fill(t.surface2).inner_margin(egui::Margin::symmetric(16, 10)).stroke(Stroke::new(1.0, t.line))).show(ui, |ui| {
            match self.progress {
                None => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
                        ui.label(RichText::new("Nothing running. Pick a model above to start.").size(13.0).color(t.fg3));
                    });
                }
                Some(p) => {
                    ui.label(RichText::new("Downloads & imports").size(12.0).color(t.fg3));
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&self.name).font(mono(13.0)));
                        ui.add_space(8.0);
                        let (r, _) = ui.allocate_exact_size(vec2(320.0, 6.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, t.line);
                        let mut f = r;
                        f.set_width(320.0 * p);
                        ui.painter().rect_filled(f, 3.0, if p >= 1.0 { t.ok } else { t.accent });
                        let status = if p >= 1.0 { "Ready in Ollama".to_string() } else if self.paused { format!("Paused {:.0}%", p * 100.0) } else { format!("Downloading {:.0}%", p * 100.0) };
                        ui.label(RichText::new(status).font(mono(12.0)).color(if p >= 1.0 { t.ok } else { t.fg2 }));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if p >= 1.0 {
                                if small_button(ui, "Dismiss") { self.progress = None; }
                            } else {
                                if small_button(ui, "Cancel") { self.progress = None; self.paused = false; }
                                if small_button(ui, if self.paused { "Resume" } else { "Pause" }) { self.paused = !self.paused; }
                            }
                        });
                    });
                }
            }
        });

        // filters
        egui::Panel::left("filters").exact_size(200.0).resizable(false).frame(egui::Frame::new().fill(t.surface2).inner_margin(16.0).stroke(Stroke::new(1.0, t.line))).show(ui, |ui| {
            caption(ui, "Format");
            let (mut a, mut b, mut c) = (true, true, false);
            ui.checkbox(&mut a, "GGUF");
            ui.checkbox(&mut b, "Safetensors");
            ui.checkbox(&mut c, "PyTorch / other");
            ui.add_space(10.0);
            caption(ui, "Fits my machine");
            let mut f = true;
            ui.checkbox(&mut f, "Only models that fit");
            ui.label(RichText::new("Detected 16 GB VRAM, 32 GB RAM").size(12.0).color(t.fg3));
            ui.add_space(10.0);
            caption(ui, "Task");
            let (mut g, mut e) = (true, false);
            ui.checkbox(&mut g, "Text generation");
            ui.checkbox(&mut e, "Embeddings");
        });

        if self.tab != 0 {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(24.0)).show(ui, |ui| {
                ui.label(RichText::new("Not part of this spike. See the HTML mockup for this view.").color(t.fg3));
            });
            return;
        }

        // results
        egui::Panel::left("results").default_size(350.0).size_range(300.0..=420.0).frame(egui::Frame::new().fill(t.surface).stroke(Stroke::new(1.0, t.line))).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(format!("{} of 1,284 results", MODELS.len())).size(12.0).color(t.fg3));
            });
            ui.add_space(-8.0);
            for (i, m) in MODELS.iter().enumerate() {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 88.0), Sense::click());
                let selected = i == self.sel;
                if selected {
                    ui.painter().rect_filled(rect, 0.0, t.accent_soft);
                    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, t.accent);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, t.surface2);
                }
                ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.line));
                ui.scope_builder(UiBuilder::new().max_rect(rect.shrink2(vec2(16.0, 10.0))), |ui| {
                    ui.spacing_mut().item_spacing.y = 6.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new(format!("{}/", m.owner)).color(t.fg3));
                        ui.add(egui::Label::new(RichText::new(m.name).font(semibold(14.0))).truncate());
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        if m.safetensors { tag(ui, "Safetensors", t.warn, t.warn_soft) } else { tag(ui, "GGUF", t.ok, t.ok_soft) }
                        tag(ui, m.params, t.fg2, t.surface2);
                        tag(ui, if m.arch.len() > 14 { "Mistral…" } else { m.arch }, t.fg2, t.surface2);
                    });
                    ui.label(RichText::new(format!("↓ {}     {}", m.dl, m.upd)).size(12.0).color(t.fg3));
                });
                if resp.clicked() {
                    self.select_model(i);
                }
            }
        });

        // detail
        let m = &MODELS[self.sel];
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface).inner_margin(egui::Margin::symmetric(24, 20))).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.heading(m.name);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("by {} · {} downloads · updated {}", m.owner, m.dl, m.upd)).color(t.fg3));
                    ui.hyperlink_to(RichText::new("Open on huggingface.co ↗").size(12.0), format!("https://huggingface.co/{}/{}", m.owner, m.name));
                });
                ui.add_space(10.0);

                // metadata strip
                egui::Frame::new().stroke(Stroke::new(1.0, t.line)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                    let total = ui.available_width();
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (frac, k, v) in [(0.18, "Parameters", m.params), (0.38, "Architecture", m.arch), (0.2, "Context", m.ctx), (0.24, "License", m.lic)] {
                            ui.vertical(|ui| {
                                ui.set_width(total * frac);
                                caption(ui, k);
                                ui.add(egui::Label::new(RichText::new(v).font(mono(13.0)).strong()).wrap_mode(egui::TextWrapMode::Extend));
                            });
                        }
                    });
                });
                ui.add_space(12.0);

                if m.safetensors {
                    egui::Frame::new().fill(t.warn_soft).stroke(Stroke::new(1.0, t.warn)).corner_radius(8.0).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Safetensors, supported by Ollama. Weights download in full precision (14.5 GB) and Ollama converts and quantizes them when it creates the model.").size(13.0));
                    });
                    ui.add_space(6.0);
                }
                caption(ui, if m.safetensors { "Choose a quantization" } else { "Choose a file" });

                // quantization table, painted by hand
                let w = ui.available_width();
                let cx = |f: f32| f * w;
                let (head, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                ui.painter().rect_filled(head, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, t.surface2);
                for (x, label, right) in [(cx(0.03), "QUANTIZATION", false), (cx(0.41), "SIZE", true), (cx(0.47), "QUALITY", false), (cx(0.80), "ON YOUR MACHINE", false)] {
                    ui.painter().text(egui::pos2(head.min.x + x, head.center().y), if right { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }, label, semibold(11.0), t.fg3);
                }
                let n = m.quants.len();
                let mut clicked = None;
                ui.spacing_mut().item_spacing.y = 0.0;
                for (i, q) in m.quants.iter().enumerate() {
                    let (row, resp) = ui.allocate_exact_size(vec2(w, 38.0), Sense::click());
                    let sel = i == self.quant;
                    if sel {
                        ui.painter().rect_filled(row, 0.0, t.accent_soft);
                        ui.painter().rect_filled(Rect::from_min_size(row.min, vec2(3.0, row.height())), 0.0, t.accent);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(row, 0.0, t.surface2);
                    }
                    if i + 1 < n {
                        ui.painter().hline(row.x_range(), row.bottom(), Stroke::new(1.0, t.line));
                    }
                    let cy = row.center().y;
                    let name_rect = ui.painter().text(egui::pos2(row.min.x + cx(0.03), cy), Align2::LEFT_CENTER, q.0, mono(13.5), t.fg);
                    if q.4 {
                        ui.painter().text(egui::pos2(name_rect.right() + 10.0, cy), Align2::LEFT_CENTER, "Recommended", semibold(11.0), t.accent);
                    }
                    ui.painter().text(egui::pos2(row.min.x + cx(0.41), cy), Align2::RIGHT_CENTER, format!("{:.2} GB", q.1), mono(13.0), t.fg);
                    let bar_r = Rect::from_min_size(egui::pos2(row.min.x + cx(0.47), cy - 3.0), vec2(64.0, 6.0));
                    ui.painter().rect_filled(bar_r, 3.0, if sel { t.surface } else { t.line });
                    ui.painter().rect_filled(Rect::from_min_size(bar_r.min, vec2(64.0 * q.3, 6.0)), 3.0, t.accent);
                    let note_w = (row.min.x + cx(0.80) - 12.0) - (bar_r.right() + 10.0);
                    let mut job = egui::text::LayoutJob::single_section(q.2.into(), egui::TextFormat::simple(FontId::proportional(12.0), t.fg2));
                    job.wrap = egui::text::TextWrapping { max_width: note_w, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                    let note = ui.painter().layout_job(job);
                    ui.painter().galley(egui::pos2(bar_r.right() + 10.0, cy - note.size().y / 2.0), note, t.fg2);
                    let (label, kind) = fit_of(q.1);
                    let (fg, bg) = if kind == Fit::Ok { (t.ok, t.ok_soft) } else { (t.warn, t.warn_soft) };
                    let g = ui.painter().layout_no_wrap(label.into(), FontId::proportional(12.0), fg);
                    let pill = Rect::from_min_size(egui::pos2(row.min.x + cx(0.80), cy - 10.0), vec2(g.size().x + 16.0, 20.0));
                    ui.painter().rect_filled(pill, 10.0, if sel { t.surface } else { bg });
                    ui.painter().galley(pill.center() - g.size() / 2.0, g, fg);
                    if resp.clicked() {
                        clicked = Some(i);
                    }
                }
                let table_bottom = ui.min_rect().bottom();
                let table = Rect::from_min_max(egui::pos2(head.min.x, head.min.y), egui::pos2(head.max.x, table_bottom));
                ui.painter().rect_stroke(table, 8.0, Stroke::new(1.0, t.line), StrokeKind::Inside);
                if let Some(i) = clicked {
                    self.quant = i;
                    self.rename();
                }
                ui.spacing_mut().item_spacing.y = 6.0;
                ui.add_space(14.0);

                // import plan card
                egui::Frame::new().fill(t.surface2).stroke(Stroke::new(1.0, t.line)).corner_radius(8.0).inner_margin(14.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    caption(ui, "Model name in Ollama");
                    let size = if m.safetensors { 14.5 } else { m.quants[self.quant].1 };
                    ui.horizontal(|ui| {
                        let btn_w = 230.0;
                        ui.add_sized(vec2(ui.available_width() - btn_w - 14.0, 36.0), egui::TextEdit::singleline(&mut self.name).font(mono(13.0)).margin(egui::Margin::symmetric(10, 9)));
                        if primary_button(ui, &format!("Download & import · {size:.2} GB"), self.progress.is_none()) {
                            self.progress = Some(0.0);
                            self.paused = false;
                        }
                    });
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                        let steps = if m.safetensors { ["Download", "Verify sha256", "Upload blobs", "Quantize", "Delete staging"] } else { ["Download", "Verify sha256", "Upload blob", "Create model", "Delete staging"] };
                        for (i, s) in steps.iter().enumerate() {
                            if i > 0 {
                                ui.label(RichText::new("›").color(t.fg3));
                            }
                            tag(ui, s, t.fg2, t.surface);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new("Temporary space").size(12.0).color(t.fg3));
                        ui.label(RichText::new(format!("{size:.2} GB")).font(mono(12.0)));
                        ui.add_space(12.0);
                        ui.label(RichText::new("Final in Ollama").size(12.0).color(t.fg3));
                        ui.label(RichText::new(format!("{:.2} GB", m.quants[self.quant].1)).font(mono(12.0)));
                        ui.add_space(12.0);
                        ui.label(RichText::new("Staging free").size(12.0).color(t.fg3));
                        ui.label(RichText::new("412 GB ✓").font(mono(12.0)).color(t.ok));
                    });
                });
            });
        });
    }
}

fn main() -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1180.0, 820.0]).with_title("hf2ollama (egui spike)"),
        ..Default::default()
    };
    eframe::run_native("hf2ollama", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

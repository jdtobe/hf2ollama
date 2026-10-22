#![allow(dead_code)]
mod runtime;
mod theme;
mod widgets;

use eframe::egui;

struct App;

impl eframe::App for App {
    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}
}

fn main() -> eframe::Result<()> {
    eframe::run_native(
        "hf2ollama",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(App))),
    )
}

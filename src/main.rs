use std::sync::mpsc::channel;

use anyhow::Result;
use memoni::opengl_context::OpenGLContext;
use memoni::selection::Selection;
use memoni::x11_window::X11Window;
use memoni::{input::Input, utils::is_plaintext_mime};
use x11rb::connection::Connection;
use x11rb::protocol::Event;

fn main() -> Result<()> {
    let width = 420u16;
    let height = 550u16;
    let background_color = 0x191919;

    let window = X11Window::new(width, height, background_color)?;
    let mut gl_context = unsafe { OpenGLContext::new(&window)? };
    let mut input = Input::new(&window)?;
    let mut selection = Selection::new(&window)?;
    let egui_ctx = egui::Context::default();

    let (ctrlc_tx, ctrlc_rx) = channel();
    ctrlc::set_handler(move || {
        ctrlc_tx
            .send(())
            .expect("Could not send signal on channel.")
    })?;

    window.map_window()?;
    window.grab_input()?;

    'main_loop: while ctrlc_rx.try_recv().is_err() {
        while let Ok(Some(event)) = window.conn.poll_for_event() {
            if let Event::Error(_) = event {
                continue;
            }

            if let Event::DestroyNotify(_) = event {
                break 'main_loop;
            }

            input.handle_event(&event);
            selection.handle_event(&event)?;
        }

        let mut selection_items = vec![];
        for item in &selection.items {
            if let Some((_, value)) = item.data.iter().find(|(k, _)| is_plaintext_mime(k)) {
                selection_items.push(str::from_utf8(value)?);
            }
        }

        let mut quit = false;
        let full_output = egui_ctx.run(input.egui_input.take(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Hello World!");
                if ui.button("Quit").clicked() {
                    quit = true;
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, item) in selection_items.iter().enumerate() {
                        ui.label(format!("{}: {}", i, item));
                    }
                });
            });
        });

        if quit {
            break;
        }

        gl_context.render(&egui_ctx, full_output)?;
    }

    window.ungrab_input()?;

    Ok(())
}

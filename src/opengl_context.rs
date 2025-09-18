use crate::{config::Config, x11_window::X11Window};
use anyhow::{Context as _, Result};
use egui::Color32;
use egui_glow::Painter;
use glow::Context as GlowContext;
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext},
    display::Display,
    prelude::{GlDisplay as _, NotCurrentGlContext as _},
    surface::{GlSurface as _, Surface, SurfaceAttributesBuilder, WindowSurface},
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, XcbDisplayHandle, XcbWindowHandle};
use std::{
    ffi::CString,
    num::{NonZero, NonZeroU32},
    ptr::NonNull,
    sync::Arc,
};

pub struct OpenGLContext {
    pub display: Display,
    pub dimensions: [u32; 2],
    pub background: (f32, f32, f32),
    pub surface: Surface<WindowSurface>,
    pub context: PossiblyCurrentContext,
    pub gl: Arc<GlowContext>,
    pub painter: Painter,
}

impl OpenGLContext {
    pub unsafe fn new(window: &X11Window, config: &Config) -> Result<Self> {
        let display_handle = XcbDisplayHandle::new(
            NonNull::new(window.conn.get_raw_xcb_connection()),
            window.screen_num as _,
        );
        let window_handle = XcbWindowHandle::new(NonZero::new(window.win_id).unwrap());

        // TODO: switch to glx for transparency
        let gl_display = unsafe {
            Display::new(
                RawDisplayHandle::Xcb(display_handle),
                glutin::display::DisplayApiPreference::Egl,
            )?
        };

        let config_template_builder = ConfigTemplateBuilder::new()
            .prefer_hardware_accelerated(None)
            .with_depth_size(0)
            .with_stencil_size(0)
            .with_transparency(true);

        let display_config = unsafe {
            gl_display
                .find_configs(config_template_builder.build())?
                .next()
                .context("No suitable config found")?
        };

        let attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(glutin::context::Version::new(
                3, 3,
            ))))
            .build(Some(RawWindowHandle::Xcb(window_handle)));

        let surface_attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            RawWindowHandle::Xcb(window_handle),
            NonZero::new(config.layout.window_dimensions.width)
                .unwrap()
                .into(),
            NonZero::new(config.layout.window_dimensions.height)
                .unwrap()
                .into(),
        );

        let surface = unsafe { gl_display.create_window_surface(&display_config, &surface_attrs)? };

        let ctx: PossiblyCurrentContext = unsafe {
            gl_display
                .create_context(&display_config, &attrs)?
                .make_current(&surface)?
        };

        surface.set_swap_interval(&ctx, glutin::surface::SwapInterval::Wait(NonZeroU32::MIN))?;

        let gl = unsafe {
            GlowContext::from_loader_function(|s| {
                gl_display.get_proc_address(CString::new(s).unwrap().as_c_str())
            })
        };
        let gl = Arc::new(gl);

        let painter = Painter::new(gl.clone(), "", None, true)?;

        let background_color: Color32 = config.theme.background.into();
        let (r, g, b, _) = background_color.to_tuple();

        Ok(OpenGLContext {
            display: gl_display,
            dimensions: [
                config.layout.window_dimensions.width as _,
                config.layout.window_dimensions.height as _,
            ],
            background: (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0),
            surface,
            context: ctx,
            gl,
            painter,
        })
    }

    pub fn render(
        &mut self,
        egui_ctx: &egui::Context,
        full_output: egui::FullOutput,
    ) -> Result<()> {
        let egui::FullOutput {
            platform_output: _,
            mut textures_delta,
            mut shapes,
            pixels_per_point,
            viewport_output: _,
        } = full_output;

        let (r, g, b) = self.background;
        unsafe {
            use glow::HasContext as _;
            self.gl.clear_color(r, g, b, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }

        let mut textures_delta = std::mem::take(&mut textures_delta);
        for (id, image_delta) in textures_delta.set {
            self.painter.set_texture(id, &image_delta);
        }

        let shapes = std::mem::take(&mut shapes);
        let clipped_primitives = egui_ctx.tessellate(shapes, pixels_per_point);
        self.painter
            .paint_primitives(self.dimensions, pixels_per_point, &clipped_primitives);

        for id in textures_delta.free.drain(..) {
            self.painter.free_texture(id);
        }

        self.surface.swap_buffers(&self.context)?;
        Ok(())
    }
}

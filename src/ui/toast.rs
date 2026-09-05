use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event_loop::ActiveEventLoop;
use winit::platform::windows::WindowAttributesExtWindows;
use winit::window::{Theme, Window, WindowId, WindowLevel};

use crate::ui::text::TextRenderer;
use crate::ui::{Canvas, Rect};

const TOAST_W: u32 = 320;
const TOAST_H: u32 = 80;
const DISPLAY_DURATION: Duration = Duration::from_millis(2500);

#[derive(Clone, Debug)]
pub enum ToastKind {
    Ocr(String),
    FullScreenshot,
}

pub struct Toast {
    window: Rc<Window>,
    _context: softbuffer::Context<Rc<Window>>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    expires_at: Instant,
    scale: f64,
    kind: ToastKind,
    dark: bool,
}

impl Toast {
    pub fn new(
        event_loop: &ActiveEventLoop,
        kind: ToastKind,
        renderer: Option<&TextRenderer>,
    ) -> Option<Self> {
        let (pos, scale) = calculate_position(event_loop);

        let attrs = Window::default_attributes()
            .with_title("pashari notification")
            .with_decorations(false)
            .with_resizable(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_skip_taskbar(true)
            .with_active(false)
            // Creating the toast hidden and calling set_visible(true) later
            // does not reliably show it on Windows/winit.
            .with_visible(true)
            .with_position(pos)
            .with_inner_size(LogicalSize::new(
                TOAST_W as f64,
                TOAST_H as f64,
            ));

        let window = Rc::new(event_loop.create_window(attrs).ok()?);

        let dark = window
            .theme()
            .map(|theme| theme == Theme::Dark)
            .unwrap_or(true);

        let _ = window.set_cursor_hittest(false);
        crate::overlay::exclude_from_capture(&window);
        crate::overlay::disable_window_animations(&window);

        // Ensure the window does not activate or steal focus when created or clicked.
        if let Ok(handle) = window.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
                    WS_EX_TOOLWINDOW,
                };

                unsafe {
                    let hwnd = HWND(h.hwnd.get() as *mut core::ffi::c_void);
                    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE);

                    SetWindowLongW(
                        hwnd,
                        GWL_EXSTYLE,
                        ex | WS_EX_NOACTIVATE.0 as i32 | WS_EX_TOOLWINDOW.0 as i32,
                    );
                }
            }
        }

        let context = softbuffer::Context::new(window.clone()).ok()?;
        let mut surface = softbuffer::Surface::new(&context, window.clone()).ok()?;

        let size = window.inner_size();
        let (cw, ch) = (size.width.max(1), size.height.max(1));

        surface
            .resize(
                NonZeroU32::new(cw)?,
                NonZeroU32::new(ch)?,
            )
            .ok()?;

        let mut toast = Self {
            window,
            _context: context,
            surface,
            expires_at: Instant::now() + DISPLAY_DURATION,
            scale,
            kind,
            dark,
        };

        toast.render(renderer);
        toast.window.request_redraw();

        Some(toast)
    }

    pub fn update(&mut self, kind: ToastKind, renderer: Option<&TextRenderer>) {
        self.kind = kind;
        self.expires_at = Instant::now() + DISPLAY_DURATION;

        self.render(renderer);
        self.window.request_redraw();
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub fn owns_window(&self, id: WindowId) -> bool {
        self.window.id() == id
    }

    pub fn render(&mut self, renderer: Option<&TextRenderer>) {
        let Some(renderer) = renderer else {
            return;
        };

        let Ok(mut buffer) = self.surface.buffer_mut() else {
            eprintln!("[toast] buffer_mut 失敗");
            return;
        };

        let size = self.window.inner_size();
        let (w, h) = (size.width as usize, size.height as usize);

        if w == 0 || h == 0 {
            return;
        }

        let mut canvas = Canvas {
            buf: &mut buffer,
            w,
            h,
            scale: self.scale,
        };

        // Keep the toast visually consistent with the Settings window.
        //
        // Dark:
        //   BG   = 0x001E1E1E
        //   BORDER / FIELD-ish dark tones
        //   TEXT = 0x00EAEAEA
        //   DIM  = 0x00999999
        //
        // Light:
        //   BG   = 0x00F5F5F5
        //   BORDER = 0x00E0E0E0
        //   TEXT = 0x00202020
        //   DIM  = 0x00707070
        let (bg_color, border_color, message_color, title_color) = if self.dark {
            (
                0x001E_1E1E,
                0x0033_3333,
                0x00EA_EAEA,
                0x0099_9999,
            )
        } else {
            (
                0x00F5_F5F5,
                0x00E0_E0E0,
                0x0020_2020,
                0x0070_7070,
            )
        };

        canvas.fill(
            Rect {
                x0: 0,
                y0: 0,
                x1: TOAST_W as usize,
                y1: TOAST_H as usize,
            },
            bg_color,
        );

        canvas.stroke(
            Rect {
                x0: 0,
                y0: 0,
                x1: TOAST_W as usize,
                y1: TOAST_H as usize,
            },
            border_color,
        );

        let (accent_color, title, message) = match &self.kind {
            ToastKind::FullScreenshot => (
                // Emerald green works on both themes.
                0x0034_D399,
                "Fullscreen screenshot",
                "Saved screenshot".to_string(),
            ),

            ToastKind::Ocr(raw_text) => {
                let cleaned = raw_text
                    .replace(['\r', '\n'], " ")
                    .trim()
                    .to_string();

                let text = if cleaned.is_empty() {
                    "テキストは検出されませんでした".to_string()
                } else {
                    cleaned
                };

                (
                    // Sky blue, matching the Settings blue accent family.
                    if self.dark {
                        0x0038_BDF8
                    } else {
                        0x0026_8CCB
                    },
                    "OCR result",
                    text,
                )
            }
        };

        // Accent bar.
        canvas.fill(
            Rect {
                x0: 3,
                y0: 10,
                x1: 6,
                y1: (TOAST_H - 10) as usize,
            },
            accent_color,
        );

        // Title.
        let title_size = 19.0;
        let title_baseline =
            renderer.baseline_for_center(22.0, title_size);

        renderer.draw(
            &mut canvas,
            16.0,
            title_baseline,
            title,
            title_size,
            title_color,
        );

        // Message.
        let msg_size = 21.0;
        let max_text_width = (TOAST_W - 28) as f32;

        let mut display_msg = message;

        if renderer.text_width(&display_msg, msg_size) > max_text_width {
            while !display_msg.is_empty()
                && renderer.text_width(
                    &format!("{display_msg}..."),
                    msg_size,
                ) > max_text_width
            {
                display_msg.pop();
            }

            display_msg.push_str("...");
        }

        let msg_baseline =
            renderer.baseline_for_center(52.0, msg_size);

        renderer.draw(
            &mut canvas,
            16.0,
            msg_baseline,
            &display_msg,
            msg_size,
            message_color,
        );

        if let Err(e) = buffer.present() {
            eprintln!("[toast] buffer.present 失敗: {e}");
        }
    }
}

/// Computes the bottom-right position for the toast above the taskbar
/// on the primary monitor.
fn calculate_position(
    event_loop: &ActiveEventLoop,
) -> (PhysicalPosition<i32>, f64) {
    let (mx, my, mw, mh, scale) = event_loop
        .primary_monitor()
        .map(|m| {
            let pos = m.position();
            let size = m.size();

            (
                pos.x,
                pos.y,
                size.width as i32,
                size.height as i32,
                m.scale_factor(),
            )
        })
        .unwrap_or((0, 0, 1920, 1080, 1.0));

    let win_w_phys =
        (TOAST_W as f64 * scale).round() as i32;
    let win_h_phys =
        (TOAST_H as f64 * scale).round() as i32;

    let margin_x =
        (20.0 * scale).round() as i32;
    let margin_y =
        (16.0 * scale).round() as i32;

    // Try reading the work area (excluding taskbar) via Win32.
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW,
        SPI_GETWORKAREA,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    };

    let mut work_area = RECT::default();

    let success = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work_area as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };

    let (x, y) = if success.is_ok()
        && work_area.right > work_area.left
    {
        (
            work_area.right
                - win_w_phys
                - margin_x,
            work_area.bottom
                - win_h_phys
                - margin_y,
        )
    } else {
        (
            mx + mw - win_w_phys - margin_x,
            my + mh
                - win_h_phys
                - (margin_y
                    + (48.0 * scale).round() as i32),
        )
    };

    (PhysicalPosition::new(x, y), scale)
}

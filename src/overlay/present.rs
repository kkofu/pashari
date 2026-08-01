//! Presents the overlay's CPU-rendered frame through a DXGI flip-model
//! swapchain.
//!
//! The rest of the app presents with softbuffer, which blits through GDI
//! (`BitBlt`). GDI has no way to synchronize that copy against DWM's
//! composition, so DWM can sample a half-copied surface and show a tear.
//! That's tolerable for the small, rarely-repainted Settings/Editor
//! windows, but not for this overlay: it covers the whole virtual desktop
//! and repaints every pixel on every cursor move while zoomed. A flip
//! swapchain hands DWM whole finished buffers instead, so a torn frame
//! can't be composited in the first place.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_STRETCH,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::core::Interface;

pub struct FlipPresenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swapchain: IDXGISwapChain1,
    /// A GPU-side staging texture the CPU frame is uploaded into, then
    /// copied to the backbuffer. Uploading straight into the backbuffer
    /// isn't allowed for a flip-model swapchain.
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
}

impl FlipPresenter {
    /// Sets up a D3D11 device and a flip-model swapchain on `hwnd`.
    /// Returns `Err` if anything is unavailable, so the caller can fall
    /// back to softbuffer rather than failing the capture entirely.
    pub fn new(hwnd: HWND, width: u32, height: u32) -> Result<Self, String> {
        let (width, height) = (width.max(1), height.max(1));

        // SAFETY: standard D3D11/DXGI initialization. Every out-param is a
        // fresh Option/uninit local, and each call's HRESULT is checked
        // before the resulting interface is used. `hwnd` is a live winit
        // window handle owned by the caller for longer than this presenter.
        unsafe {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            // BGRA_SUPPORT is required for a B8G8R8A8 swapchain. Falls back
            // to the WARP software renderer so the overlay still works on
            // machines without a usable GPU driver.
            let mut result = Err(windows::core::Error::empty());
            for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
                result = D3D11CreateDevice(
                    None,
                    driver,
                    None,
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                );
                if result.is_ok() {
                    break;
                }
            }
            result.map_err(|e| format!("D3D11 device creation failed: {e}"))?;
            let device = device.ok_or("D3D11 returned no device")?;
            let context = context.ok_or("D3D11 returned no device context")?;

            let factory: IDXGIFactory2 =
                CreateDXGIFactory1().map_err(|e| format!("DXGI factory creation failed: {e}"))?;
            let dxgi_device: IDXGIDevice = device
                .cast()
                .map_err(|e| format!("D3D11 device is not a DXGI device: {e}"))?;

            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                Stereo: false.into(),
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                // Flip model needs at least 2 buffers; 2 is enough since
                // each present is a full repaint with nothing queued behind it.
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                // The overlay window is opaque (not layered), so DWM
                // doesn't need per-pixel alpha from us.
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                Flags: 0,
            };
            let swapchain = factory
                .CreateSwapChainForHwnd(&dxgi_device, hwnd, &desc, None, None)
                .map_err(|e| format!("swapchain creation failed: {e}"))?;
            // Keeps DXGI from hijacking Alt+Enter into exclusive fullscreen.
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);

            let staging = create_staging(&device, width, height)?;

            Ok(Self {
                device,
                context,
                swapchain,
                staging,
                width,
                height,
            })
        }
    }

    /// Uploads `pixels` (0x00RRGGBB, row-major, `width * height` long) and
    /// presents it, blocking until the next vblank.
    pub fn present(&mut self, pixels: &[u32]) -> Result<(), String> {
        let expected = self.width as usize * self.height as usize;
        if pixels.len() < expected {
            return Err(format!(
                "frame buffer too small: {} < {expected}",
                pixels.len()
            ));
        }

        // SAFETY: `staging` is sized `width * height` in the same format as
        // the backbuffer, and `pixels` is verified above to hold at least
        // that many u32s, so the row pitch below stays in bounds. The
        // backbuffer is fetched fresh each frame (flip model invalidates
        // the previous one after Present) and dropped at the end of scope.
        unsafe {
            self.context.UpdateSubresource(
                &self.staging,
                0,
                None,
                pixels.as_ptr().cast(),
                self.width * 4,
                0,
            );
            let back: ID3D11Texture2D = self
                .swapchain
                .GetBuffer(0)
                .map_err(|e| format!("swapchain GetBuffer failed: {e}"))?;
            self.context.CopyResource(&back, &self.staging);
            // SyncInterval 1: waits for the next vblank and hands DWM a
            // complete buffer — this is what actually removes the tearing.
            self.swapchain
                .Present(1, DXGI_PRESENT(0))
                .ok()
                .map_err(|e| format!("swapchain Present failed: {e}"))?;
        }
        Ok(())
    }

    /// Resizes the swapchain and staging texture to match a new window
    /// size. No-op if unchanged.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        let (width, height) = (width.max(1), height.max(1));
        if width == self.width && height == self.height {
            return Ok(());
        }
        // SAFETY: no backbuffer reference is outstanding here (`present`
        // drops its own before returning), which is ResizeBuffers'
        // precondition. The staging texture is replaced wholesale.
        unsafe {
            self.swapchain
                .ResizeBuffers(
                    0,
                    width,
                    height,
                    DXGI_FORMAT_B8G8R8A8_UNORM,
                    DXGI_SWAP_CHAIN_FLAG(0),
                )
                .map_err(|e| format!("swapchain ResizeBuffers failed: {e}"))?;
            self.staging = create_staging(&self.device, width, height)?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }
}

/// A DEFAULT-usage texture matching the backbuffer's format, used as the
/// upload target for the CPU frame.
///
/// # Safety
/// `device` must be a live D3D11 device.
unsafe fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex: Option<ID3D11Texture2D> = None;
    // SAFETY: `desc` is fully initialized above and `tex` is a fresh local
    // the callee writes into; the HRESULT is checked before it's unwrapped.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex)) }
        .map_err(|e| format!("staging texture creation failed: {e}"))?;
    tex.ok_or_else(|| "staging texture creation returned nothing".to_string())
}

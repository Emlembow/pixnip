//! Windows.Graphics.Capture backend: captures a monitor into an explicit
//! scRGB FP16 surface so our own SDR-white-level tone mapping runs, instead
//! of accepting the compositor's naive HDR->SDR conversion. This is the
//! same API the modern Snipping Tool uses, and it honors FP16 even on
//! drivers that reject `IDXGIOutput5::DuplicateOutput1`.

use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{E_FAIL, HMODULE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16G16B16A16_FLOAT;
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter1, IDXGIDevice, IDXGIOutput, DXGI_OUTPUT_DESC};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use crate::capture::{
    bgra8_to_rgba8, fp16_scrgb_to_rgba8, sdr_white_scale_for_output, CapturedMonitor,
};

/// Captures one DXGI output via Windows.Graphics.Capture at FP16 and
/// tone-maps it against the monitor's SDR white level.
pub unsafe fn capture_output(
    adapter: &IDXGIAdapter1,
    output: &IDXGIOutput,
) -> windows::core::Result<CapturedMonitor> {
    let desc: DXGI_OUTPUT_DESC = output.GetDesc()?;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    D3D11CreateDevice(
        adapter,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT,
        None,
        D3D11_SDK_VERSION,
        Some(&mut device),
        None,
        Some(&mut context),
    )?;
    let device = device.ok_or(windows::core::Error::from(E_FAIL))?;
    let context = context.ok_or(windows::core::Error::from(E_FAIL))?;

    let dxgi_device: IDXGIDevice = device.cast()?;
    let inspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?;
    let winrt_device: IDirect3DDevice = inspectable.cast()?;

    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = interop.CreateForMonitor(desc.Monitor)?;
    let size = item.Size()?;

    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &winrt_device,
        DirectXPixelFormat::R16G16B16A16Float,
        2,
        size,
    )?;
    let session = frame_pool.CreateCaptureSession(&item)?;
    // Not fatal if unsupported (older Windows builds).
    let _ = session.SetIsCursorCaptureEnabled(false);
    let _ = session.SetIsBorderRequired(false);
    session.StartCapture()?;

    // Free-threaded pool delivers frames on a worker thread; poll for one.
    let deadline = Instant::now() + Duration::from_millis(2000);
    let frame = loop {
        match frame_pool.TryGetNextFrame() {
            Ok(frame) => break frame,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => {
                let _ = session.Close();
                let _ = frame_pool.Close();
                return Err(e);
            }
        }
    };

    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let texture: ID3D11Texture2D = access.GetInterface()?;

    let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
    texture.GetDesc(&mut tex_desc);
    let width = tex_desc.Width;
    let height = tex_desc.Height;
    let format = tex_desc.Format;

    tex_desc.Usage = D3D11_USAGE_STAGING;
    tex_desc.BindFlags = 0;
    tex_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    tex_desc.MiscFlags = 0;

    let mut staging: Option<ID3D11Texture2D> = None;
    device.CreateTexture2D(&tex_desc, None, Some(&mut staging))?;
    let staging = staging.ok_or(windows::core::Error::from(E_FAIL))?;

    context.CopyResource(&staging, &texture);

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

    // On an SDR desktop the white level reports 1000 (scale 1.0) and scRGB
    // values stay <= 1.0, so the FP16 path is correct there too.
    let sdr_scale = if format == DXGI_FORMAT_R16G16B16A16_FLOAT {
        sdr_white_scale_for_output(&desc).unwrap_or(1.0)
    } else {
        1.0
    };

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let row_pitch = mapped.RowPitch as usize;
    let data = std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * height as usize);

    if format == DXGI_FORMAT_R16G16B16A16_FLOAT {
        fp16_scrgb_to_rgba8(data, row_pitch, width, height, sdr_scale, &mut rgba);
    } else {
        bgra8_to_rgba8(data, row_pitch, width, height, &mut rgba);
    }

    context.Unmap(&staging, 0);
    let _ = session.Close();
    let _ = frame_pool.Close();

    Ok(CapturedMonitor {
        x: desc.DesktopCoordinates.left,
        y: desc.DesktopCoordinates.top,
        width,
        height,
        rgba,
    })
}

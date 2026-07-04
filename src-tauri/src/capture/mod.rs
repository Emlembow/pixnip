pub mod tonemap;
pub mod wgc;

use std::mem::size_of;
use windows::core::Interface;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL,
    DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R16G16B16A16_FLOAT,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput1, IDXGIResource,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC,
};

pub struct CapturedMonitor {
    /// This monitor's origin in virtual desktop coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Tone-mapped, display-correct sRGB pixels, row-major, 4 bytes/pixel.
    pub rgba: Vec<u8>,
}

impl CapturedMonitor {
    /// Returns a new capture cropped to the given region (in this monitor's
    /// local pixel coordinates, i.e. already offset by `x`/`y`).
    pub fn cropped(&self, x: u32, y: u32, width: u32, height: u32) -> CapturedMonitor {
        let mut rgba = vec![0u8; (width * height * 4) as usize];
        for row in 0..height {
            let src_start = (((y + row) * self.width + x) * 4) as usize;
            let src_end = src_start + (width * 4) as usize;
            let dst_start = (row * width * 4) as usize;
            let dst_end = dst_start + (width * 4) as usize;
            rgba[dst_start..dst_end].copy_from_slice(&self.rgba[src_start..src_end]);
        }
        CapturedMonitor {
            x: self.x + x as i32,
            y: self.y + y as i32,
            width,
            height,
            rgba,
        }
    }
}

/// Captures every active monitor, preferring Windows.Graphics.Capture with
/// an explicit scRGB FP16 surface (so *our* SDR-white-level tone mapping
/// runs, not the compositor's naive one). Falls back per-monitor to legacy
/// DXGI Desktop Duplication where WGC is unavailable.
pub fn capture_all_monitors() -> windows::core::Result<Vec<CapturedMonitor>> {
    let mut results = Vec::new();
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;

        let mut adapter_index = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            adapter_index += 1;

            let mut output_index = 0u32;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                output_index += 1;
                match wgc::capture_output(&adapter, &output) {
                    Ok(captured) => results.push(captured),
                    Err(wgc_err) => {
                        eprintln!("WGC capture failed ({wgc_err:?}); using DXGI duplication");
                        if let Ok(captured) = capture_output_dxgi(&adapter, &output) {
                            results.push(captured);
                        }
                    }
                }
            }
        }
    }
    Ok(results)
}

unsafe fn capture_output_dxgi(
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
    let device = device.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;
    let context = context.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;

    let output1: IDXGIOutput1 = output.cast()?;
    let duplication = output1.DuplicateOutput(&device)?;

    let dup_desc = duplication.GetDesc();
    let width = dup_desc.ModeDesc.Width;
    let height = dup_desc.ModeDesc.Height;

    // The very first frame after (re)starting duplication can come back
    // blank on some drivers; keep acquiring (releasing in between) until we
    // get a frame with real accumulated content, or give up.
    let mut resource: Option<IDXGIResource> = None;
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut attempts = 0;
    loop {
        match duplication.AcquireNextFrame(200, &mut frame_info, &mut resource) {
            Ok(()) => {
                if frame_info.AccumulatedFrames > 0 || attempts >= 9 {
                    break;
                }
                duplication.ReleaseFrame()?;
                resource = None;
                attempts += 1;
                continue;
            }
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT && attempts < 9 => {
                attempts += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    let resource = resource.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;
    let texture: ID3D11Texture2D = resource.cast()?;

    let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
    texture.GetDesc(&mut tex_desc);
    tex_desc.Usage = D3D11_USAGE_STAGING;
    tex_desc.BindFlags = 0;
    tex_desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    tex_desc.MiscFlags = 0;

    let mut staging: Option<ID3D11Texture2D> = None;
    device.CreateTexture2D(&tex_desc, None, Some(&mut staging))?;
    let staging = staging.ok_or(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))?;

    context.CopyResource(&staging, &texture);

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
    let format = tex_desc.Format;

    let sdr_scale = sdr_white_scale_for_output(&desc).unwrap_or(1.0);

    let mut rgba = vec![0u8; (width * height * 4) as usize];
    let row_pitch = mapped.RowPitch as usize;
    let data = std::slice::from_raw_parts(mapped.pData as *const u8, row_pitch * height as usize);

    match format {
        DXGI_FORMAT_R16G16B16A16_FLOAT => {
            fp16_scrgb_to_rgba8(data, row_pitch, width, height, sdr_scale, &mut rgba);
        }
        DXGI_FORMAT_B8G8R8A8_UNORM => {
            bgra8_to_rgba8(data, row_pitch, width, height, &mut rgba);
        }
        _ => {
            return Err(windows::core::Error::from(
                windows::Win32::Foundation::E_FAIL,
            ))
        }
    }

    context.Unmap(&staging, 0);
    duplication.ReleaseFrame()?;

    Ok(CapturedMonitor {
        x: desc.DesktopCoordinates.left,
        y: desc.DesktopCoordinates.top,
        width,
        height,
        rgba,
    })
}

/// Lookup table for the linear -> sRGB gamma encode, so the per-pixel hot
/// loop avoids `powf`. Indexed by linear value scaled to [0, 4095].
fn srgb_lut() -> &'static [u8; 4096] {
    static LUT: std::sync::OnceLock<[u8; 4096]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0u8; 4096];
        for (i, entry) in lut.iter_mut().enumerate() {
            *entry = tonemap::linear_to_srgb_u8(i as f32 / 4095.0);
        }
        lut
    })
}

/// Converts a mapped FP16 scRGB surface into display-correct 8-bit sRGB
/// via the SDR-white-level tone map. Shared by the WGC and DXGI paths.
/// Rows are processed across all cores — this touches 7M+ pixels on a
/// large monitor and sits directly on the keypress-to-overlay latency.
pub(crate) fn fp16_scrgb_to_rgba8(
    data: &[u8],
    row_pitch: usize,
    width: u32,
    height: u32,
    sdr_scale: f32,
    rgba: &mut [u8],
) {
    let lut = srgb_lut();
    let width = width as usize;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let rows_per_chunk = (height as usize).div_ceil(threads);

    std::thread::scope(|scope| {
        for (chunk_index, out_chunk) in rgba.chunks_mut(rows_per_chunk * width * 4).enumerate() {
            let start_row = chunk_index * rows_per_chunk;
            scope.spawn(move || {
                for (row_offset, out_row) in out_chunk.chunks_mut(width * 4).enumerate() {
                    let y = start_row + row_offset;
                    let row = &data[y * row_pitch..y * row_pitch + width * 8];
                    for x in 0..width {
                        let px = &row[x * 8..x * 8 + 8];
                        let r = half::f16::from_bits(u16::from_le_bytes([px[0], px[1]])).to_f32();
                        let g = half::f16::from_bits(u16::from_le_bytes([px[2], px[3]])).to_f32();
                        let b = half::f16::from_bits(u16::from_le_bytes([px[4], px[5]])).to_f32();
                        let [r, g, b] = tonemap::tonemap_pixel([r, g, b], sdr_scale);
                        let out = x * 4;
                        out_row[out] = lut[(r.clamp(0.0, 1.0) * 4095.0) as usize];
                        out_row[out + 1] = lut[(g.clamp(0.0, 1.0) * 4095.0) as usize];
                        out_row[out + 2] = lut[(b.clamp(0.0, 1.0) * 4095.0) as usize];
                        out_row[out + 3] = 255;
                    }
                }
            });
        }
    });
}

/// Swizzles a mapped BGRA8 surface (already display-referred) to RGBA8.
pub(crate) fn bgra8_to_rgba8(
    data: &[u8],
    row_pitch: usize,
    width: u32,
    height: u32,
    rgba: &mut [u8],
) {
    for y in 0..height as usize {
        let row = &data[y * row_pitch..y * row_pitch + width as usize * 4];
        for x in 0..width as usize {
            let px = &row[x * 4..x * 4 + 4];
            let out = (y * width as usize + x) * 4;
            rgba[out] = px[2];
            rgba[out + 1] = px[1];
            rgba[out + 2] = px[0];
            rgba[out + 3] = 255;
        }
    }
}

/// Looks up the real SDR white level for the monitor behind `desc` and
/// returns the scale factor that normalizes scRGB values against it.
/// Falls back to `Ok(1.0)` semantics (via caller's `unwrap_or`) if the
/// monitor can't be matched or the OS doesn't report a level for it.
pub(crate) unsafe fn sdr_white_scale_for_output(
    desc: &DXGI_OUTPUT_DESC,
) -> windows::core::Result<f32> {
    let device_name = pwstr_to_string(&desc.DeviceName);

    let mut path_count = 0u32;
    let mut mode_count = 0u32;
    if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count).0 != 0 {
        return Ok(1.0);
    }

    let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![Default::default(); path_count as usize];
    let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![Default::default(); mode_count as usize];
    if QueryDisplayConfig(
        QDC_ONLY_ACTIVE_PATHS,
        &mut path_count,
        paths.as_mut_ptr(),
        &mut mode_count,
        modes.as_mut_ptr(),
        None,
    )
    .0 != 0
    {
        return Ok(1.0);
    }
    paths.truncate(path_count as usize);

    for path in &paths {
        let mut source_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source_name.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source_name.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source_name.header.adapterId = path.sourceInfo.adapterId;
        source_name.header.id = path.sourceInfo.id;
        if DisplayConfigGetDeviceInfo(&mut source_name.header) != 0 {
            continue;
        }
        if pwstr_to_string(&source_name.viewGdiDeviceName) != device_name {
            continue;
        }

        let mut white_level = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
        white_level.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        white_level.header.size = size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32;
        white_level.header.adapterId = path.targetInfo.adapterId;
        white_level.header.id = path.targetInfo.id;
        if DisplayConfigGetDeviceInfo(&mut white_level.header) == 0 {
            return Ok(tonemap::sdr_scale_from_white_level(
                white_level.SDRWhiteLevel,
            ));
        }
    }

    Ok(1.0)
}

fn pwstr_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}
